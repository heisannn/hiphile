//! What the tests of several handles share. Apart from [`crate::test_support`], which the views'
//! tests use too, as `wait_for` needs GPUI, a dev-dependency only.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::time::Duration;

use gpui_kit::TestAppContext;
use serde_json::json;

/// A server that cannot be reached.
pub(crate) const NOWHERE: &str = "http://127.0.0.1:9";

/// A server that gives `fields` in answer to anything.
pub(crate) fn server_answering(fields: serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while request.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let mut body = json!({ "status": "ok", "version": "1.16.1" });
            body.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let body = json!({ "subsonic-response": body }).to_string();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len(),
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(body.as_bytes()).ok();
        }
    });
    url
}

/// Runs what is under way until `done`, as answers from the network arrive off the executor.
pub(crate) fn wait_for(cx: &mut TestAppContext, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("what was waited for never happened");
}
