//! A server of covers, for the tests and the shots of what shows them.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Answers `getCoverArt` with one PNG, whatever cover is asked for, and anything else with 404.
pub struct CoverServer {
    url: String,
    asked: Arc<AtomicUsize>,
}

impl CoverServer {
    pub fn new(png: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = asked.clone();
        let png: Arc<[u8]> = png.into();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut head = [0; 4096];
                let Ok(read) = stream.read(&mut head) else {
                    continue;
                };
                let head = String::from_utf8_lossy(&head[..read]);
                let target = head.split(' ').nth(1).unwrap_or_default();
                if target.contains("/getCoverArt") {
                    counted.fetch_add(1, Ordering::SeqCst);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        png.len()
                    );
                    let _ = stream.write_all(&png);
                } else {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        });
        Self { url, asked }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// How many covers have been asked for.
    pub fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

/// A PNG of `size` pixels across and down, each of them `pixel(x, y)` in RGBA.
pub fn png(size: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let rgba: Vec<u8> = (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .flat_map(|(x, y)| pixel(x, y))
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cover.png");
    anti_webui::app::headless::write_png(&path, size, size, &rgba).unwrap();
    std::fs::read(path).unwrap()
}
