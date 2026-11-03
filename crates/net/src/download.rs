use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use futures::channel::oneshot;
use tokio::sync::Notify;

use crate::Error;

const CANCELED: &str = "canceled";
const RUNTIME_STOPPED: &str = "the network runtime stopped";

/// A response body that is still arriving, readable as a blocking
/// [`Read`] + [`Seek`] source.
///
/// Reads past the bytes received so far block until more arrive, the body
/// ends, the network fails or times out, or the download is canceled through
/// a [`DownloadHandle`]. Use it from a thread that may block (a decoder
/// thread, the CLI), never from a UI executor. The whole body is kept in
/// memory. Dropping it cancels the download.
pub struct Download {
    shared: Arc<Shared>,
    pos: u64,
    content_type: Option<String>,
    content_length: Option<u64>,
    finished: Option<oneshot::Receiver<()>>,
}

/// Cancels a [`Download`] from another thread: a blocked read returns an
/// error and the connection is closed. Bytes already received stay readable.
#[derive(Clone)]
pub struct DownloadHandle {
    shared: Arc<Shared>,
}

impl DownloadHandle {
    pub fn cancel(&self) {
        self.shared.cancel();
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    /// Wakes the network task. `notify_one` keeps a permit, so a cancel that
    /// arrives between two awaits is not lost.
    cancel: Notify,
}

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    /// `None` while the body is still arriving. Set once, never replaced.
    outcome: Option<Result<(), String>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Records how the body ended, unless it already has, and wakes readers.
    fn finish(&self, outcome: Result<(), String>) {
        let mut state = self.lock();
        if state.outcome.is_none() {
            state.outcome = Some(outcome);
        }
        drop(state);
        self.changed.notify_all();
    }

    fn cancel(&self) {
        self.finish(Err(CANCELED.to_owned()));
        self.cancel.notify_one();
    }
}

impl Download {
    /// The `Content-Type` header, e.g. `audio/flac`.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// The `Content-Length` header, when the server sent one.
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    pub fn handle(&self) -> DownloadHandle {
        DownloadHandle {
            shared: self.shared.clone(),
        }
    }

    /// Waits for the whole body. For small documents, not media.
    pub async fn into_body(mut self) -> Result<Bytes, Error> {
        if let Some(finished) = self.finished.take() {
            finished.await.ok();
        }
        let state = self.shared.lock();
        match &state.outcome {
            Some(Ok(())) => Ok(Bytes::copy_from_slice(&state.bytes)),
            Some(Err(message)) => Err(Error::Interrupted(message.clone())),
            None => Err(Error::Canceled),
        }
    }

    /// Total length: the header if present, otherwise known once complete.
    fn wait_for_len(&self) -> io::Result<u64> {
        if let Some(len) = self.content_length {
            return Ok(len);
        }
        let mut state = self.shared.lock();
        loop {
            match &state.outcome {
                Some(Ok(())) => return Ok(state.bytes.len() as u64),
                Some(Err(message)) => return Err(io::Error::other(message.clone())),
                None => state = self.shared.wait(state),
            }
        }
    }
}

impl Read for Download {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut state = self.shared.lock();
        loop {
            let available = state.bytes.len() as u64;
            if self.pos < available {
                let start = self.pos as usize;
                let len = buf.len().min(state.bytes.len() - start);
                buf[..len].copy_from_slice(&state.bytes[start..start + len]);
                self.pos += len as u64;
                return Ok(len);
            }
            match &state.outcome {
                Some(Ok(())) => return Ok(0),
                // Not `ErrorKind::Interrupted`: `read_to_end` and decoders
                // retry that kind, which would turn a cancel into a busy loop.
                Some(Err(message)) => return Err(io::Error::other(message.clone())),
                None => state = self.shared.wait(state),
            }
        }
    }
}

impl Seek for Download {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
            SeekFrom::End(delta) => self.wait_for_len()?.checked_add_signed(delta),
        };
        let target = target
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        self.pos = target;
        Ok(target)
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.shared.cancel();
    }
}

/// Settles the download however the network task ends, including when its
/// future is dropped mid-body because the runtime shut down. Without it a
/// reader would wait for an outcome that never comes.
struct Completion {
    shared: Arc<Shared>,
    finished: Option<oneshot::Sender<()>>,
}

impl Drop for Completion {
    fn drop(&mut self) {
        // No effect when the task already recorded its own outcome.
        self.shared.finish(Err(RUNTIME_STOPPED.to_owned()));
        if let Some(finished) = self.finished.take() {
            finished.send(()).ok();
        }
    }
}

/// Runs on the network runtime: reports the headers, then appends the body.
pub(crate) async fn run(
    request: impl Future<Output = Result<reqwest::Response, reqwest::Error>>,
    headers: oneshot::Sender<Result<Download, Error>>,
) {
    let mut response = match request.await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            headers
                .send(Err(Error::HttpStatus(response.status().as_u16())))
                .ok();
            return;
        }
        Err(error) => {
            headers.send(Err(error.into())).ok();
            return;
        }
    };

    let shared = Arc::new(Shared::default());
    let (finished, finished_receiver) = oneshot::channel();
    let completion = Completion {
        shared: shared.clone(),
        finished: Some(finished),
    };
    let download = Download {
        shared: shared.clone(),
        pos: 0,
        content_type: response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        content_length: response.content_length(),
        finished: Some(finished_receiver),
    };
    if headers.send(Ok(download)).is_err() {
        return;
    }

    let outcome = loop {
        tokio::select! {
            () = shared.cancel.notified() => break Err(CANCELED.to_owned()),
            chunk = response.chunk() => match chunk {
                Ok(Some(chunk)) => {
                    shared.lock().bytes.extend_from_slice(&chunk);
                    shared.changed.notify_all();
                }
                Ok(None) => break Ok(()),
                Err(error) => break Err(error.without_url().to_string()),
            },
        }
    };
    shared.finish(outcome);
    drop(completion);
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    fn download(bytes: &[u8], done: bool) -> (Download, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        {
            let mut state = shared.lock();
            state.bytes.extend_from_slice(bytes);
            state.outcome = done.then_some(Ok(()));
        }
        let download = Download {
            shared: shared.clone(),
            pos: 0,
            content_type: None,
            content_length: None,
            finished: None,
        };
        (download, shared)
    }

    #[test]
    fn reads_then_reports_end() {
        let (mut download, _) = download(b"abc", true);
        let mut buf = Vec::new();
        download.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"abc");
    }

    #[test]
    fn read_waits_for_more_bytes() {
        let (mut download, shared) = download(b"ab", false);
        let writer = std::thread::spawn(move || {
            shared.lock().bytes.extend_from_slice(b"cd");
            shared.finish(Ok(()));
        });
        let mut buf = Vec::new();
        download.read_to_end(&mut buf).unwrap();
        writer.join().unwrap();
        assert_eq!(buf, b"abcd");
    }

    #[test]
    fn seek_from_end_waits_for_length() {
        let (mut download, _) = download(b"abcdef", true);
        assert_eq!(download.seek(SeekFrom::End(-2)).unwrap(), 4);
        let mut buf = [0; 2];
        download.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ef");
    }

    #[test]
    fn interrupted_body_is_a_read_error() {
        let (mut download, shared) = download(b"ab", false);
        shared.finish(Err("reset".to_owned()));
        let mut buf = Vec::new();
        assert!(download.read_to_end(&mut buf).is_err());
    }

    #[test]
    fn seek_before_start_is_rejected() {
        let (mut download, _) = download(b"ab", true);
        assert!(download.seek(SeekFrom::Current(-1)).is_err());
    }

    #[test]
    fn a_finished_outcome_is_never_replaced() {
        let (download, shared) = download(b"ab", true);
        download.handle().cancel();
        assert!(matches!(shared.lock().outcome, Some(Ok(()))));
    }

    /// A server that announces 10 bytes, sends 3, then stalls. Reports on
    /// the returned channel when the client closes the connection.
    fn stalling_server() -> (url::Url, mpsc::Receiver<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let (closed, closed_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nContent-Length: 10\r\n\r\nabc",
                )
                .unwrap();
            let mut rest = [0; 1];
            // Returns once the client closes or resets the connection.
            let _ = reader.read(&mut rest);
            closed.send(()).ok();
        });
        (url, closed_receiver)
    }

    /// Reads to the end on another thread; the channel yields its result.
    fn read_in_background(mut download: Download) -> mpsc::Receiver<io::Result<Vec<u8>>> {
        let (result, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            result
                .send(download.read_to_end(&mut buf).map(|_| buf))
                .ok();
        });
        receiver
    }

    const WAIT: Duration = Duration::from_secs(5);

    fn start(client: &crate::Client, url: url::Url) -> Download {
        futures::executor::block_on(client.download(url)).unwrap()
    }

    #[test]
    fn cancel_wakes_a_blocked_reader_and_closes_the_connection() {
        let (url, closed) = stalling_server();
        let client = crate::Client::new("test").unwrap();
        let download = start(&client, url);
        let handle = download.handle();
        let result = read_in_background(download);
        std::thread::sleep(Duration::from_millis(100));
        handle.cancel();
        assert!(result.recv_timeout(WAIT).unwrap().is_err());
        closed.recv_timeout(WAIT).expect("connection still open");
    }

    #[test]
    fn stopping_the_runtime_wakes_a_blocked_reader() {
        let (url, _closed) = stalling_server();
        let client = crate::Client::new("test").unwrap();
        let result = read_in_background(start(&client, url));
        std::thread::sleep(Duration::from_millis(100));
        drop(client);
        assert!(result.recv_timeout(WAIT).unwrap().is_err());
    }

    #[test]
    fn a_stalled_body_times_out() {
        let (url, _closed) = stalling_server();
        let client = crate::Client::with_timeouts(
            "test",
            Duration::from_secs(1),
            Duration::from_millis(300),
        )
        .unwrap();
        let result = read_in_background(start(&client, url));
        assert!(result.recv_timeout(WAIT).unwrap().is_err());
    }

    #[test]
    fn into_body_rejects_a_body_cut_short() {
        let (url, _closed) = stalling_server();
        let client = crate::Client::new("test").unwrap();
        let download = start(&client, url);
        download.handle().cancel();
        assert!(futures::executor::block_on(download.into_body()).is_err());
    }
}
