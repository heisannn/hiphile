use std::net::TcpListener;

use super::*;
use crate::testing::{NOWHERE, core, start, wait_until};

/// A server that takes each request and never answers it.
fn server_never_answering() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let _requests: Vec<_> = listener.incoming().collect();
    });
    url
}

fn cover_url(server: &str, id: &str) -> Arc<str> {
    Arc::from(format!("{server}/rest/getCoverArt?id={id}&size=96"))
}

#[test]
fn views_of_one_cover_share_its_fetch() {
    let (_core, hiphile) = core("");
    let url = cover_url(&server_never_answering(), "al-1");
    let shown = signal(2usize);
    let _ui = start({
        let url = url.clone();
        move || {
            install_covers(hiphile);
            div().child(for_each(
                move || (0..shown.get()).collect(),
                |i| *i,
                move |_| cover(Some(url.clone()), CoverKind::Album).size(48),
            ))
        }
    });
    assert_eq!(known(), [(url.clone(), 2, true)]);

    shown.set(1);
    assert_eq!(known(), [(url.clone(), 1, true)]);
}

#[test]
fn a_fetch_no_view_waits_for_is_called_off() {
    let (_core, hiphile) = core("");
    let url = cover_url(&server_never_answering(), "al-1");
    let on = signal(true);
    let _ui = start({
        let url = url.clone();
        move || {
            install_covers(hiphile);
            div().child(show(
                move || on.get(),
                move || cover(Some(url.clone()), CoverKind::Album),
            ))
        }
    });

    on.set(false);

    assert!(known().is_empty());
}

#[test]
fn a_cover_the_server_has_not_stays_a_placeholder_and_stays_known() {
    let (_core, hiphile) = core("");
    let url = cover_url(NOWHERE, "al-1");
    let on = signal(true);
    let mut ui = start({
        let url = url.clone();
        move || {
            install_covers(hiphile);
            div().child(show(
                move || on.get(),
                move || cover(Some(url.clone()), CoverKind::Album),
            ))
        }
    });

    wait_until(&mut ui, || known() == [(url.clone(), 1, false)]);
    on.set(false);

    assert_eq!(known(), [(url, 0, false)]);
}
