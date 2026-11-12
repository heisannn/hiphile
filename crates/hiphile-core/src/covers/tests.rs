use std::sync::Arc;

use super::FetchedCovers;

fn image(bytes: &[u8]) -> Arc<[u8]> {
    Arc::from(bytes)
}

fn held(fetched: &FetchedCovers) -> Vec<String> {
    let mut urls: Vec<String> = fetched
        .0
        .lock()
        .unwrap()
        .keys()
        .map(ToString::to_string)
        .collect();
    urls.sort();
    urls
}

#[test]
fn the_same_image_fetched_again_is_the_one_still_held() {
    let fetched = FetchedCovers::default();
    let first = fetched.share("a".into(), image(b"cover"));

    let again = fetched.share("a".into(), image(b"cover"));
    assert!(Arc::ptr_eq(&first, &again));

    let changed = fetched.share("a".into(), image(b"other"));
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(&*changed, b"other");
}

#[test]
fn an_image_no_one_holds_goes_when_any_is_fetched() {
    let fetched = FetchedCovers::default();
    let kept = fetched.share("a".into(), image(b"cover"));
    drop(fetched.share("b".into(), image(b"gone")));

    let again = fetched.share("a".into(), image(b"cover"));

    assert!(Arc::ptr_eq(&kept, &again));
    assert_eq!(held(&fetched), ["a"]);
}
