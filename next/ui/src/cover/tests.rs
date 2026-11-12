use std::net::TcpListener;

use anti_webui::core::Ui;
use anti_webui::core::tree::{NodeKind, with_doc};
use hiphile_core::test_support::LibraryServer;

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

fn png() -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cover.png");
    anti_webui::app::headless::write_png(&path, 2, 2, &[128; 16]).unwrap();
    std::fs::read(path).unwrap()
}

fn shows_a_picture() -> bool {
    with_doc(|d| {
        d.nodes
            .values()
            .any(|node| matches!(node.kind, NodeKind::Image(_)))
    })
}

/// A cover of `url` while `on` says so.
fn shown_while(hiphile: hiphile_core::Hiphile, url: Arc<str>, on: Signal<bool>) -> Ui {
    start(move || {
        install_covers(hiphile);
        div().child(show(
            move || on.get(),
            move || cover(Some(url.clone()), CoverKind::Album),
        ))
    })
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
fn a_cover_found_shows_as_its_picture_and_stays_known() {
    let server = LibraryServer::new(Vec::new());
    server.serve_covers(|_| Some(png()));
    server.hold("getCoverArt");
    let (_core, hiphile) = core("");
    let url = cover_url(server.url(), "al-1");
    let on = signal(true);
    let mut ui = shown_while(hiphile, url.clone(), on);
    assert!(!shows_a_picture());

    server.release("getCoverArt");
    wait_until(&mut ui, shows_a_picture);
    assert_eq!(known(), [(url.clone(), 1, false)]);
    on.set(false);

    assert_eq!(known(), [(url, 0, false)]);
}

#[test]
fn a_cover_that_could_not_be_had_is_asked_for_again_once_shown_again() {
    let (_core, hiphile) = core("");
    let url = cover_url(NOWHERE, "al-1");
    let on = signal(true);
    let mut ui = shown_while(hiphile, url.clone(), on);
    wait_until(&mut ui, || known() == [(url.clone(), 1, false)]);
    assert!(!shows_a_picture());

    on.set(false);
    assert!(known().is_empty());
    on.set(true);

    assert_eq!(known(), [(url, 1, true)]);
}
