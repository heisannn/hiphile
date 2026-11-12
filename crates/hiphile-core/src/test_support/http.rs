//! What the fake servers share: a port to listen on, the requests they read and the answers they
//! write.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::thread;

use serde_json::{Value, json};

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Request {
    pub verb: String,
    pub method: String,
    /// From the query, then from a form, in their order and decoded.
    pub(super) params: Vec<(String, String)>,
}

impl Request {
    pub fn values(&self, name: &str) -> Vec<&str> {
        self.params
            .iter()
            .filter(|(param, _)| param == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    pub(super) fn value(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Serves each connection to a new local port on a thread of its own, and gives the port's URL.
pub(super) fn listen<S: Clone + Send + 'static>(state: S, serve: fn(TcpStream, &S)) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            thread::spawn(move || serve(stream, &state));
        }
    });
    url
}

pub(super) fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    let body_start = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = end + 4;
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .map_or(0, |length| length.trim().parse().unwrap_or(0));
            if bytes.len() >= end + length {
                break end;
            }
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&bytes[..body_start]);
    let mut request_line = head.lines().next()?.split(' ');
    let verb = request_line.next()?.to_owned();
    let target = request_line.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let method = path
        .rsplit('/')
        .next()?
        .trim_end_matches(".view")
        .to_owned();
    let form = String::from_utf8_lossy(&bytes[body_start..]);
    let params = query
        .split('&')
        .chain(form.split('&'))
        .filter_map(|param| param.split_once('='))
        .map(|(name, value)| (decoded(name), decoded(value)))
        .collect();
    Some(Request {
        verb,
        method,
        params,
    })
}

/// An answer that went well, to add a payload to.
pub(super) fn ok() -> Value {
    json!({ "status": "ok", "version": "1.16.1" })
}

pub(super) fn fail(body: &mut Value, code: u32, message: &str) {
    body["status"] = json!("failed");
    body["error"] = json!({ "code": code, "message": message });
}

/// The payload's fields alongside those of [`ok`], or a failure with code 0 and the message.
pub(super) fn answered(answer: Result<Value, &str>) -> Value {
    let mut body = ok();
    match answer {
        Ok(Value::Object(payload)) => body.as_object_mut().unwrap().extend(payload),
        Ok(_) => {}
        Err(message) => fail(&mut body, 0, message),
    }
    body
}

pub(super) fn write_json(stream: &mut TcpStream, body: Value) {
    let body = json!({ "subsonic-response": body }).to_string();
    write(stream, "200 OK", Some("application/json"), body.as_bytes());
}

/// The PNG, or Not Found without one.
pub(super) fn write_image(stream: &mut TcpStream, png: Option<&[u8]>) {
    match png {
        Some(png) => write(stream, "200 OK", Some("image/png"), png),
        None => write(stream, "404 Not Found", None, &[]),
    }
}

fn write(stream: &mut TcpStream, status: &str, content_type: Option<&str>, body: &[u8]) {
    let content_type = content_type.map_or(String::new(), |content_type| {
        format!("Content-Type: {content_type}\r\n")
    });
    let head = format!(
        "HTTP/1.1 {status}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    // A request given up on has nobody to answer
    stream.write_all(head.as_bytes()).ok();
    stream.write_all(body).ok();
}

/// `+` and `%XX` taken back to what they stand for.
fn decoded(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    let mut rest = value.bytes();
    while let Some(byte) = rest.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let hex: Vec<u8> = rest.by_ref().take(2).collect();
                let code = std::str::from_utf8(&hex)
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                bytes.extend(code.map_or(hex, |code| vec![code]));
            }
            byte => bytes.push(byte),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
