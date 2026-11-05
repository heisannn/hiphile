use super::*;

#[test]
fn a_count_with_a_total_says_how_far_along_it_is() {
    let status = Status::of_sync(SyncStage::Songs, 12_000, Some(48_000));
    assert_eq!(status.count().as_deref(), Some("12000 of 48000"));
    assert_eq!(status.percent(), Some(25.));
}

#[test]
fn a_count_without_a_total_just_counts() {
    let status = Status::of_sync(SyncStage::Albums, 500, None);
    assert_eq!(status.count().as_deref(), Some("500"));
    assert_eq!(status.percent(), None);
}

#[test]
fn nothing_saved_and_no_total_says_no_number() {
    assert_eq!(Status::of_sync(SyncStage::Artists, 0, None).count(), None);
}

#[test]
fn a_total_that_proved_too_low_is_dropped() {
    let status = Status::of_sync(SyncStage::Songs, 600, Some(500));
    assert_eq!(status.count().as_deref(), Some("600"));
    assert_eq!(status.percent(), None);
}

#[test]
fn a_download_says_how_far_along_it_is_in_percent() {
    let status = Status::of_download(&ReadingsState::Downloading(0.424)).unwrap();
    assert_eq!(status.count().as_deref(), Some("42%"));
    assert_eq!(status.percent(), Some(42.));
}

#[test]
fn a_download_that_just_started_still_shows() {
    let status = Status::of_download(&ReadingsState::Downloading(0.)).unwrap();
    assert_eq!(status.count().as_deref(), Some("0%"));
    assert_eq!(status.percent(), Some(0.));
}

#[test]
fn only_a_download_under_way_is_a_download() {
    for state in [
        ReadingsState::Off,
        ReadingsState::Checking,
        ReadingsState::Ready("dir".into()),
        ReadingsState::Failed("no".into()),
    ] {
        assert_eq!(Status::of_download(&state), None);
    }
}

#[test]
fn the_notice_goes_up_once_and_comes_down_when_quiet() {
    let mut notice = Notice::default();
    assert_eq!(notice.follow(false), None);
    assert_eq!(notice.follow(true), Some(NoticeChange::Show));
    assert_eq!(notice.follow(true), None);
    assert_eq!(notice.follow(false), Some(NoticeChange::Hide));
    assert_eq!(notice.follow(false), None);
}

#[test]
fn a_notice_taken_down_can_go_up_again() {
    let mut notice = Notice::default();
    notice.follow(true);
    notice.follow(false);
    // Taking it down closes it too, which is not the user closing it.
    notice.closed();
    assert_eq!(notice.follow(true), Some(NoticeChange::Show));
}

#[test]
fn a_notice_the_user_closed_stays_closed_until_the_library_goes_quiet() {
    let mut notice = Notice::default();
    notice.follow(true);
    notice.closed();

    assert_eq!(notice.follow(true), None);

    assert_eq!(notice.follow(false), None);
    assert_eq!(notice.follow(true), Some(NoticeChange::Show));
}
