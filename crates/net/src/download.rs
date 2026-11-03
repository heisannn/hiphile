use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::channel::oneshot;
use reqwest::StatusCode;
use reqwest::header::{self, HeaderMap};
use tokio::sync::Notify;
use url::Url;

use crate::Error;

const CANCELED: &str = "canceled";
const RUNTIME_STOPPED: &str = "the network runtime stopped";
const CUT_SHORT: &str = "the server ended the body early";
const WRONG_PART: &str = "the server sent a different part of the file than asked";
const CHANGED: &str = "the file changed on the server";

/// A read that lands ahead of the bytes arriving waits for them when they
/// should arrive within this long, about what a new response takes to
/// start; further ahead, it starts a new request at its own position.
const WORTH_WAITING: Duration = Duration::from_millis(200);
/// How far ahead a read waits before the transfer rate is known.
const READ_AHEAD: u64 = 256 * 1024;
/// How long a response must have been arriving to measure its rate.
const RATE_WINDOW: Duration = Duration::from_millis(100);
/// A read at a gap no longer than this, such as a probe of the file's end
/// for tags, gets a request of its own while the current response waits to
/// be read on afterwards. Longer gaps replace the current response.
const DETOUR_MAX: u64 = 64 * 1024;
/// Further attempts at a part of the body after a failure that may pass.
const RETRIES: u32 = 3;
/// The wait before the first retry; each retry waits longer.
const RETRY_DELAY: Duration = Duration::from_millis(500);
/// The largest body allocated up front for random access. Larger ones, or
/// a server's mistaken length, arrive in order and take only what arrives.
const MAX_RANDOM_ACCESS_LEN: u64 = 4 << 30;

/// A response body that is still arriving, readable as a blocking
/// [`Read`] + [`Seek`] source.
///
/// Reads past the bytes received so far block until they arrive, the
/// network fails or times out, or the download is canceled through a
/// [`DownloadHandle`]. When the server accepts byte ranges and reports the
/// length, as Navidrome does for original files, a read far from where the
/// body is arriving starts a new request at its own position, so a seek
/// does not wait for everything before it. The parts it skipped are fetched
/// afterwards. Other servers send the body in order and reads wait for it.
///
/// Use it from a thread that may block (a decoder thread, the CLI), never
/// from a UI executor. The whole body is kept in memory. Dropping it cancels
/// the download unless the body has all arrived; see
/// [`DownloadHandle::completed`].
pub struct Download {
    shared: Arc<Shared>,
    pos: u64,
    content_type: Option<String>,
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

    /// Waits for the download to end, then hands over the whole body if it
    /// arrived whole and as long as the `Content-Length` header said.
    ///
    /// `None` when the download was canceled or failed, including when its
    /// [`Download`] was dropped first, or when the server sent no length.
    /// The future keeps the body in memory until it is dropped. It needs no
    /// particular executor and does not block.
    pub fn completed(&self) -> impl Future<Output = Option<Bytes>> + Send + 'static {
        let shared = self.shared.clone();
        async move {
            loop {
                // Created before the check, so a finish between the two
                // still wakes it.
                let finished = shared.finished.notified();
                {
                    let state = shared.lock();
                    match &state.outcome {
                        Some(Ok(())) => {
                            return state
                                .body
                                .clone()
                                .filter(|body| shared.content_length == Some(body.len() as u64));
                        }
                        Some(Err(_)) => return None,
                        None => {}
                    }
                }
                finished.await;
            }
        }
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    /// Wakes the network task. `notify_one` keeps a permit, so a cancel that
    /// arrives between two awaits is not lost.
    cancel: Notify,
    /// Asks the network task to fetch [`State::wanted`] next.
    retarget: Notify,
    /// Wakes [`DownloadHandle::completed`] once the outcome is set.
    finished: Notify,
    /// The `Content-Length` header, when the server sent one.
    content_length: Option<u64>,
    /// The body's length, when the server sent ranges of it on request at
    /// the start and the whole body is allocated.
    len: Option<u64>,
}

#[derive(Default)]
struct State {
    /// Indexed by position in the body. A ranged download allocates the
    /// whole body up front; bytes not yet received are zero. Moved to
    /// `body` once every byte has arrived.
    bytes: Vec<u8>,
    /// The whole body, once the download has succeeded. Never written to,
    /// so it can be handed over without a copy.
    body: Option<Bytes>,
    /// The parts of `bytes` received: sorted, disjoint and never touching.
    received: Vec<Range<u64>>,
    /// What the current response is sending: from the next byte to arrive
    /// to the end of the request.
    fetching: Option<Range<u64>>,
    /// Where a reader is waiting, when the current response will not reach
    /// it soon.
    wanted: Option<u64>,
    /// Whether a read far from the bytes arriving may start a request at
    /// its own position. Turns off if the server stops sending ranges.
    random_access: bool,
    /// Bytes per second the last response arrived at, once measured.
    rate: Option<f64>,
    /// `None` while bytes are still to come. Set once, never replaced.
    outcome: Option<Result<(), String>>,
}

impl State {
    /// Stores the next bytes of the current response, ignoring any beyond
    /// what was requested. Returns whether the request is complete.
    fn receive(&mut self, chunk: &[u8]) -> bool {
        let Some(fetching) = self.fetching.clone() else {
            return true;
        };
        let len = chunk
            .len()
            .min(usize::try_from(fetching.end - fetching.start).unwrap_or(usize::MAX));
        self.write(fetching.start, &chunk[..len]);
        let next = fetching.start + len as u64;
        self.fetching = Some(next..fetching.end);
        next == fetching.end
    }

    fn write(&mut self, at: u64, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        let start = usize::try_from(at).expect("positions in memory fit in usize");
        let end = start + chunk.len();
        if self.bytes.len() < end {
            self.bytes.resize(end, 0);
        }
        self.bytes[start..end].copy_from_slice(chunk);

        let range = at..end as u64;
        let first = self.received.partition_point(|run| run.end < range.start);
        let last = self.received.partition_point(|run| run.start <= range.end);
        // Runs `first..last` overlap or touch the new bytes.
        let merged = if first < last {
            let start = self.received[first].start.min(range.start);
            start..self.received[last - 1].end.max(range.end)
        } else {
            range
        };
        self.received.splice(first..last, [merged]);
    }

    /// The body, as much of it as has arrived.
    fn bytes(&self) -> &[u8] {
        self.body.as_deref().unwrap_or(&self.bytes)
    }

    /// Copies what has arrived from `pos` into `buf`; `None` if `pos` has
    /// not arrived.
    fn copy(&self, pos: u64, buf: &mut [u8]) -> Option<usize> {
        let run = &self.received[self.received.partition_point(|run| run.end <= pos)..]
            .first()
            .filter(|run| run.start <= pos)?;
        let start = pos as usize;
        let len = buf.len().min((run.end - pos) as usize);
        buf[..len].copy_from_slice(&self.bytes()[start..start + len]);
        Some(len)
    }

    /// The first stretch from `from` on, before `len`, not received yet.
    fn first_gap(&self, from: u64, len: u64) -> Option<Range<u64>> {
        let mut pos = from;
        for run in &self.received[self.received.partition_point(|run| run.end <= from)..] {
            if run.start > pos {
                return Some(pos..run.start.min(len));
            }
            pos = run.end;
        }
        (pos < len).then_some(pos..len)
    }

    /// For a reader waiting at `pos`: whether to ask for a request there,
    /// because the current one will not get there soon.
    fn wants(&mut self, pos: u64) -> bool {
        if !self.random_access {
            return false;
        }
        let read_ahead = self.rate.map_or(READ_AHEAD, |rate| {
            (rate * WORTH_WAITING.as_secs_f64()) as u64
        });
        let arriving = self
            .fetching
            .as_ref()
            .is_some_and(|fetching| fetching.contains(&pos) && pos - fetching.start <= read_ahead);
        if arriving || self.wanted == Some(pos) {
            return false;
        }
        // The task has not taken an earlier position yet; one wake suffices.
        self.wanted.replace(pos).is_none()
    }

    /// The gap at the position a reader waits for, if any.
    fn wanted_gap(&self, len: u64) -> Option<Range<u64>> {
        self.first_gap(self.wanted?, len)
    }
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

    /// Records how the download ended, unless it already has, and wakes
    /// readers.
    fn finish(&self, outcome: Result<(), String>) {
        let mut state = self.lock();
        if state.outcome.is_none() {
            if outcome.is_ok() {
                let bytes = std::mem::take(&mut state.bytes);
                state.body = Some(Bytes::from(bytes));
            }
            state.outcome = Some(outcome);
        }
        drop(state);
        self.changed.notify_all();
        self.finished.notify_waiters();
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
        self.shared.content_length
    }

    /// Whether a read anywhere in the body is about as quick as a read where
    /// it is arriving, because the server sends ranges of it on request.
    /// Reading the end of the body then does not wait for the rest.
    pub fn is_random_access(&self) -> bool {
        self.shared.len.is_some()
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
            Some(Ok(())) => Ok(state
                .body
                .clone()
                .unwrap_or_else(|| Bytes::copy_from_slice(state.bytes()))),
            Some(Err(message)) => Err(Error::Interrupted(message.clone())),
            None => Err(Error::Canceled),
        }
    }

    /// Total length: the header if present, otherwise known once complete.
    fn wait_for_len(&self) -> io::Result<u64> {
        if let Some(len) = self.shared.content_length {
            return Ok(len);
        }
        let mut state = self.shared.lock();
        loop {
            match &state.outcome {
                Some(Ok(())) => return Ok(state.bytes().len() as u64),
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
            if let Some(len) = state.copy(self.pos, buf) {
                self.pos += len as u64;
                return Ok(len);
            }
            if self.shared.len.is_some_and(|len| self.pos >= len) {
                return Ok(0);
            }
            match &state.outcome {
                Some(Ok(())) => return Ok(0),
                // Not `ErrorKind::Interrupted`: `read_to_end` and decoders
                // retry that kind, which would turn a cancel into a busy loop.
                Some(Err(message)) => return Err(io::Error::other(message.clone())),
                None => {}
            }
            if state.wants(self.pos) {
                self.shared.retarget.notify_one();
            }
            state = self.shared.wait(state);
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

/// Runs on the network runtime: reports the headers, then receives the
/// body, with further range requests where readers need them.
pub(crate) async fn run(
    http: reqwest::Client,
    url: Url,
    headers: oneshot::Sender<Result<Download, Error>>,
) {
    let mut response = match http.get(url.clone()).send().await {
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

    let len = response.content_length().filter(|&len| {
        (1..=MAX_RANDOM_ACCESS_LEN).contains(&len)
            && usize::try_from(len).is_ok()
            && sends_ranges(response.headers())
    });
    let shared = Arc::new(Shared {
        len,
        content_length: response.content_length(),
        ..Shared::default()
    });
    {
        let mut state = shared.lock();
        state.fetching = Some(0..len.unwrap_or(u64::MAX));
        state.random_access = len.is_some();
        if let Some(len) = len {
            // Zeroed memory is mapped lazily, so this costs nothing until
            // bytes arrive.
            state.bytes = vec![0; len as usize];
        }
    }
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
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        finished: Some(finished_receiver),
    };
    if headers.send(Ok(download)).is_err() {
        return;
    }

    let outcome = match len {
        // In order, with no way to resume: any failure ends it.
        None => receive(&shared, &mut response, &mut 0)
            .await
            .map(|_| ())
            .map_err(|failure| failure.message),
        Some(len) => {
            let file = File {
                http: &http,
                url: &url,
                len,
                version: Version::of(response.headers()),
            };
            file.fetch(&shared, response).await
        }
    };
    shared.finish(outcome);
    drop(completion);
}

/// Why a response or request stopped short.
struct Failure {
    message: String,
    /// Whether trying the same part again may succeed.
    transient: bool,
}

impl Failure {
    fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: true,
        }
    }

    fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: false,
        }
    }
}

enum Received {
    /// The response sent all that was asked of it, or the body ended.
    Complete,
    /// A reader waits somewhere the response will not reach soon.
    Retarget,
}

/// Stores the body of `response`, until it ends or, for random access,
/// until it has sent what was asked or a reader wants another position.
async fn receive(
    shared: &Shared,
    response: &mut reqwest::Response,
    failures: &mut u32,
) -> Result<Received, Failure> {
    let started = Instant::now();
    let mut received = 0;
    loop {
        let chunk = tokio::select! {
            () = shared.cancel.notified() => return Err(Failure::fatal(CANCELED)),
            () = shared.retarget.notified(), if shared.len.is_some() => {
                // A wake left over from a position already taken up.
                let state = shared.lock();
                if state.random_access && state.wanted.is_some() {
                    return Ok(Received::Retarget);
                }
                continue;
            }
            chunk = response.chunk() => chunk
                .map_err(|error| Failure::transient(error.without_url().to_string()))?,
        };
        let Some(chunk) = chunk else {
            // A ranged response that ends before its part did is cut short.
            return match shared.len {
                Some(_) => Err(Failure::transient(CUT_SHORT)),
                None => Ok(Received::Complete),
            };
        };
        *failures = 0;
        received += chunk.len();
        let elapsed = started.elapsed();
        let mut state = shared.lock();
        let complete = state.receive(&chunk);
        if elapsed >= RATE_WINDOW {
            state.rate = Some(received as f64 / elapsed.as_secs_f64());
        }
        drop(state);
        shared.changed.notify_all();
        if complete {
            return Ok(Received::Complete);
        }
    }
}

/// What identifies the file's version in a response, compared across the
/// responses of one download. Not sent as `If-Range`: behind some caching
/// proxies a date there never matches, and every request would get the
/// whole file.
#[derive(Default)]
struct Version {
    etag: Option<header::HeaderValue>,
    last_modified: Option<header::HeaderValue>,
}

impl Version {
    fn of(headers: &HeaderMap) -> Self {
        Self {
            etag: headers
                .get(header::ETAG)
                .filter(|etag| !etag.as_bytes().starts_with(b"W/"))
                .cloned(),
            last_modified: headers.get(header::LAST_MODIFIED).cloned(),
        }
    }

    /// Whether `headers` name another version. A header a response leaves
    /// out says nothing.
    fn differs(&self, headers: &HeaderMap) -> bool {
        let other = Self::of(headers);
        let differs = |ours: &Option<header::HeaderValue>, theirs: Option<header::HeaderValue>| {
            ours.as_ref()
                .zip(theirs)
                .is_some_and(|(ours, theirs)| *ours != theirs)
        };
        differs(&self.etag, other.etag) || differs(&self.last_modified, other.last_modified)
    }
}

/// What the network task does next for a random-access download.
enum Step {
    Receive(reqwest::Response),
    Request(Range<u64>),
}

/// A response put aside for a detour, and the part it was sending.
struct Parked {
    response: reqwest::Response,
    part: Range<u64>,
}

/// A ranged download's source: where to request parts of it, and what it
/// must still look like.
struct File<'a> {
    http: &'a reqwest::Client,
    url: &'a Url,
    len: u64,
    version: Version,
}

impl File<'_> {
    /// Receives `first`, then requests the parts readers wait for, then the
    /// rest, until the whole body has arrived.
    async fn fetch(&self, shared: &Shared, first: reqwest::Response) -> Result<(), String> {
        let mut step = Step::Receive(first);
        let mut parked = None;
        let mut failures = 0;
        loop {
            step = match step {
                Step::Receive(mut response) => {
                    match receive(shared, &mut response, &mut failures).await {
                        Ok(Received::Complete) => match self.next(shared, &mut parked) {
                            Some(step) => step,
                            None => return Ok(()),
                        },
                        Ok(Received::Retarget) => {
                            let state = shared.lock();
                            let detour = parked.is_none()
                                && state
                                    .wanted_gap(self.len)
                                    .is_some_and(|gap| gap.end - gap.start <= DETOUR_MAX);
                            if detour && let Some(part) = state.fetching.clone() {
                                parked = Some(Parked { response, part });
                            }
                            drop(state);
                            match self.next(shared, &mut parked) {
                                Some(step) => step,
                                None => return Ok(()),
                            }
                        }
                        Err(failure) => {
                            back_off(shared, failure, &mut failures).await?;
                            // The rest of the same part.
                            match shared.lock().fetching.clone() {
                                Some(part) if !part.is_empty() => Step::Request(part),
                                _ => match self.next(shared, &mut parked) {
                                    Some(step) => step,
                                    None => return Ok(()),
                                },
                            }
                        }
                    }
                }
                Step::Request(part) => {
                    let result = tokio::select! {
                        () = shared.cancel.notified() => Err(Failure::fatal(CANCELED)),
                        result = self.request(shared, part.clone()) => result,
                    };
                    match result {
                        Ok(response) => {
                            if !shared.lock().random_access {
                                parked = None;
                            }
                            Step::Receive(response)
                        }
                        Err(failure) => {
                            back_off(shared, failure, &mut failures).await?;
                            Step::Request(part)
                        }
                    }
                }
            };
        }
    }

    /// Picks what to fetch after a response stops: where a reader waits,
    /// else the rest of a parked response, else the next gap after the
    /// last part, else the first gap. `None` once the whole body is here.
    fn next(&self, shared: &Shared, parked: &mut Option<Parked>) -> Option<Step> {
        let mut state = shared.lock();
        if let Some(gap) = state
            .wanted
            .take()
            .and_then(|pos| state.first_gap(pos, self.len))
        {
            state.fetching = Some(gap.clone());
            return Some(Step::Request(gap));
        }
        if let Some(Parked { response, part }) = parked.take() {
            let still_missing = state
                .first_gap(part.start, part.end)
                .is_some_and(|gap| gap.start == part.start);
            if still_missing {
                state.fetching = Some(part);
                return Some(Step::Receive(response));
            }
        }
        let after = state.fetching.as_ref().map_or(0, |part| part.start);
        let gap = state
            .first_gap(after, self.len)
            .or_else(|| state.first_gap(0, self.len));
        state.fetching = gap.clone();
        gap.map(Step::Request)
    }

    /// Requests `part`, and points [`State::fetching`] at what the response
    /// will send: possibly less than asked, or the whole body if the server
    /// no longer sends ranges.
    async fn request(
        &self,
        shared: &Shared,
        part: Range<u64>,
    ) -> Result<reqwest::Response, Failure> {
        let response = self
            .http
            .get(self.url.clone())
            .header(
                header::RANGE,
                format!("bytes={}-{}", part.start, part.end - 1),
            )
            .send()
            .await
            .map_err(|error| Failure::transient(error.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let message = format!("the server answered with HTTP {}", status.as_u16());
            let transient = status.is_server_error()
                || matches!(
                    status,
                    StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
                );
            return Err(Failure { message, transient });
        }
        if self.version.differs(response.headers()) {
            return Err(Failure::fatal(CHANGED));
        }
        if status != StatusCode::PARTIAL_CONTENT {
            // A server may ignore a range and send the whole file; take it
            // and read on in order.
            if response.content_length().is_some_and(|len| len != self.len) {
                return Err(Failure::fatal(CHANGED));
            }
            let mut state = shared.lock();
            state.random_access = false;
            state.wanted = None;
            state.fetching = Some(0..self.len);
            return Ok(response);
        }
        let sent = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| content_range(value.to_str().ok()?));
        match sent {
            Some(sent) if sent.total.is_some_and(|total| total != self.len) => {
                Err(Failure::fatal(CHANGED))
            }
            Some(sent) if sent.part.start == part.start && sent.part.end > part.start => {
                // A server may send less than asked; the rest is another
                // request.
                shared.lock().fetching = Some(part.start..sent.part.end.min(part.end));
                Ok(response)
            }
            _ => Err(Failure::fatal(WRONG_PART)),
        }
    }
}

/// Waits before another attempt after `failure`, or gives up.
async fn back_off(shared: &Shared, failure: Failure, failures: &mut u32) -> Result<(), String> {
    if !failure.transient || *failures >= RETRIES {
        return Err(failure.message);
    }
    *failures += 1;
    tokio::select! {
        () = shared.cancel.notified() => Err(CANCELED.to_owned()),
        () = tokio::time::sleep(RETRY_DELAY * *failures) => Ok(()),
    }
}

/// Whether byte offsets in requests are offsets in this body: the server
/// accepts ranges and does not compress it.
fn sends_ranges(headers: &HeaderMap) -> bool {
    let accepts = headers
        .get_all(header::ACCEPT_RANGES)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|unit| unit.trim().eq_ignore_ascii_case("bytes"));
    let encoded = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|encoding| !encoding.trim().eq_ignore_ascii_case("identity"));
    accepts && !encoded
}

/// A `Content-Range` header such as `bytes 100-199/1000`.
#[derive(Debug, PartialEq)]
struct ContentRange {
    part: Range<u64>,
    /// `None` for `*`, unknown.
    total: Option<u64>,
}

fn content_range(value: &str) -> Option<ContentRange> {
    let (unit, rest) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().ok()?;
    Some(ContentRange {
        part: start..end.checked_add(1)?,
        total: total.trim().parse().ok(),
    })
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
            state.write(0, bytes);
            state.outcome = done.then_some(Ok(()));
        }
        let download = Download {
            shared: shared.clone(),
            pos: 0,
            content_type: None,
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
            shared.lock().write(2, b"cd");
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
    fn received_parts_merge_when_they_touch() {
        let mut state = State::default();
        state.write(10, b"ab");
        state.write(0, b"xy");
        assert_eq!(state.received, [0..2, 10..12]);
        state.write(2, b"cdefghij");
        assert_eq!(state.received, vec![0..12]);
        assert_eq!(&state.bytes, b"xycdefghijab");
    }

    #[test]
    fn gaps_are_found_from_a_position() {
        let mut state = State::default();
        state.write(0, b"ab");
        state.write(5, b"cd");
        assert_eq!(state.first_gap(0, 10), Some(2..5));
        assert_eq!(state.first_gap(5, 10), Some(7..10));
        assert_eq!(state.first_gap(8, 10), Some(8..10));
        state.write(7, b"efg");
        assert_eq!(state.first_gap(5, 10), None);
    }

    #[test]
    fn only_a_read_far_from_the_body_asks_for_a_request() {
        let mut state = State {
            fetching: Some(100..10_000_000),
            random_access: true,
            ..State::default()
        };
        assert!(!state.wants(100 + READ_AHEAD));
        assert!(state.wants(50), "behind the body");
        assert!(!state.wants(50));
        assert!(!state.wants(200 + READ_AHEAD), "the wake is still pending");
        assert_eq!(state.wanted, Some(200 + READ_AHEAD));

        // At 100 kB/s, 20 kB is worth waiting for and 30 kB is not.
        let mut state = State {
            fetching: Some(0..10_000_000),
            rate: Some(100_000.0),
            random_access: true,
            ..State::default()
        };
        assert!(!state.wants(20_000));
        assert!(state.wants(30_000));

        state.wanted = None;
        state.random_access = false;
        assert!(!state.wants(5_000_000), "reads in order wait");
    }

    #[test]
    fn reads_content_range_and_accept_ranges() {
        assert_eq!(
            content_range("bytes 100-199/1000"),
            Some(ContentRange {
                part: 100..200,
                total: Some(1000)
            })
        );
        assert_eq!(
            content_range("bytes 100-199/*"),
            Some(ContentRange {
                part: 100..200,
                total: None
            })
        );
        assert_eq!(content_range("items 1-2/3"), None);
        let mut headers = HeaderMap::new();
        assert!(!sends_ranges(&headers));
        headers.insert(
            header::ACCEPT_RANGES,
            header::HeaderValue::from_static("bytes"),
        );
        assert!(sends_ranges(&headers));
        headers.insert(
            header::CONTENT_ENCODING,
            header::HeaderValue::from_static("gzip"),
        );
        assert!(!sends_ranges(&headers));
    }

    /// How [`ranged_server`] deviates from serving a file faithfully.
    #[derive(Clone)]
    struct Behaviour {
        /// Bytes a request without a range gets before it pauses.
        head: usize,
        /// How long it pauses before sending the rest; forever if `None`.
        resume_after: Option<Duration>,
        /// The file length range responses report.
        total: usize,
        /// Answer range requests with the whole file.
        ignore_ranges: bool,
        /// The most a range response sends, saying so in `Content-Range`.
        max_part: Option<usize>,
        /// How many range requests fail with HTTP 503 first.
        fail_first: usize,
    }

    impl Behaviour {
        fn new(body: &[u8]) -> Self {
            Self {
                head: 64 * 1024,
                resume_after: None,
                total: body.len(),
                ignore_ranges: false,
                max_part: None,
                fail_first: 0,
            }
        }
    }

    /// Serves `body` with ranges, the way Navidrome serves an original file,
    /// except as `behaviour` says. Reports each `Range` header.
    fn ranged_server(
        body: Arc<Vec<u8>>,
        behaviour: Behaviour,
    ) -> (url::Url, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let (ranges, ranges_receiver) = mpsc::channel();
        let failed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(socket) = socket else { return };
                let (body, behaviour, ranges, failed) = (
                    body.clone(),
                    behaviour.clone(),
                    ranges.clone(),
                    failed.clone(),
                );
                std::thread::spawn(move || {
                    serve_ranged(socket, &body, &behaviour, &ranges, &failed);
                });
            }
        });
        (url, ranges_receiver)
    }

    fn serve_ranged(
        mut socket: std::net::TcpStream,
        body: &[u8],
        behaviour: &Behaviour,
        ranges: &mpsc::Sender<String>,
        failed: &std::sync::atomic::AtomicUsize,
    ) {
        use std::sync::atomic::Ordering;

        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut range = None;
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 2 {
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("range: bytes=") {
                range = Some(value.trim().to_owned());
            }
            line.clear();
        }
        let common = "Content-Type: audio/flac\r\nAccept-Ranges: bytes\r\nConnection: close\r\n";
        let whole = format!(
            "HTTP/1.1 200 OK\r\n{common}Content-Length: {}\r\n\r\n",
            body.len()
        );
        let Some(range) = range else {
            socket.write_all(whole.as_bytes()).unwrap();
            let head = behaviour.head.min(body.len());
            socket.write_all(&body[..head]).unwrap();
            match behaviour.resume_after {
                Some(delay) => {
                    std::thread::sleep(delay);
                    socket.write_all(&body[head..]).ok();
                }
                None => {
                    let _ = reader.read(&mut [0; 1]);
                }
            }
            return;
        };
        ranges.send(range.clone()).ok();
        if failed.fetch_add(1, Ordering::Relaxed) < behaviour.fail_first {
            socket
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .ok();
            return;
        }
        if behaviour.ignore_ranges {
            socket.write_all(whole.as_bytes()).ok();
            socket.write_all(body).ok();
            return;
        }
        let (start, end) = range.split_once('-').unwrap();
        let start: usize = start.parse().unwrap();
        let end = end.parse::<usize>().map_or(body.len(), |end| end + 1);
        let end = behaviour.max_part.map_or(end, |max| end.min(start + max));
        let headers = format!(
            "HTTP/1.1 206 Partial Content\r\n{common}Content-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\n\r\n",
            end - 1,
            behaviour.total,
            end - start
        );
        socket.write_all(headers.as_bytes()).ok();
        socket.write_all(&body[start..end]).ok();
    }

    fn song_body() -> Arc<Vec<u8>> {
        Arc::new((0..2_000_000).map(|i| (i % 251) as u8).collect())
    }

    /// Reads `len` bytes at `at`, then the whole body from the start, on
    /// another thread. Fails if that takes longer than [`WAIT`].
    fn read_at_then_all(
        mut download: Download,
        at: u64,
        len: usize,
    ) -> (io::Result<Vec<u8>>, io::Result<Vec<u8>>) {
        let (result, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            download.seek(SeekFrom::Start(at)).unwrap();
            let mut buf = vec![0; len];
            let part = download.read_exact(&mut buf).map(|()| buf);
            download.seek(SeekFrom::Start(0)).unwrap();
            let mut all = Vec::new();
            let all = download.read_to_end(&mut all).map(|_| all);
            result.send((part, all)).ok();
        });
        receiver.recv_timeout(WAIT).expect("the reads waited")
    }

    fn assert_reads(body: &[u8], at: u64, (part, all): (io::Result<Vec<u8>>, io::Result<Vec<u8>>)) {
        let at = at as usize;
        assert!(part.unwrap() == body[at..at + 1000], "wrong bytes at {at}");
        assert!(all.unwrap() == body, "the whole body did not arrive");
    }

    #[test]
    fn a_far_seek_reads_without_waiting_for_the_bytes_before_it() {
        let body = song_body();
        let (url, ranges) = ranged_server(body.clone(), Behaviour::new(&body));
        let client = crate::Client::new("test").unwrap();
        let download = start(&client, url);
        assert!(download.is_random_access());
        let far = 1_500_000;
        assert_reads(&body, far, read_at_then_all(download, far, 1000));
        assert_eq!(ranges.recv().unwrap(), format!("{far}-1999999"));
    }

    #[test]
    fn a_probe_of_the_end_reads_on_from_the_first_response() {
        let body = song_body();
        let behaviour = Behaviour {
            resume_after: Some(Duration::from_millis(300)),
            ..Behaviour::new(&body)
        };
        let (url, ranges) = ranged_server(body.clone(), behaviour);
        let client = crate::Client::new("test").unwrap();
        let end = body.len() as u64 - 1000;
        assert_reads(&body, end, read_at_then_all(start(&client, url), end, 1000));
        let ranges: Vec<_> = ranges.try_iter().collect();
        assert_eq!(
            ranges,
            [format!("{end}-1999999")],
            "the first response was dropped"
        );
    }

    #[test]
    fn a_server_that_ignores_ranges_is_read_in_order() {
        let body = song_body();
        let behaviour = Behaviour {
            ignore_ranges: true,
            ..Behaviour::new(&body)
        };
        let (url, _ranges) = ranged_server(body.clone(), behaviour);
        let client = crate::Client::new("test").unwrap();
        assert_reads(
            &body,
            1_500_000,
            read_at_then_all(start(&client, url), 1_500_000, 1000),
        );
    }

    #[test]
    fn short_parts_are_continued() {
        let body = song_body();
        let behaviour = Behaviour {
            max_part: Some(100_000),
            ..Behaviour::new(&body)
        };
        let (url, ranges) = ranged_server(body.clone(), behaviour);
        let client = crate::Client::new("test").unwrap();
        assert_reads(
            &body,
            1_500_000,
            read_at_then_all(start(&client, url), 1_500_000, 1000),
        );
        assert!(ranges.try_iter().count() > 2);
    }

    #[test]
    fn a_failed_part_is_retried() {
        let body = song_body();
        let behaviour = Behaviour {
            fail_first: 1,
            ..Behaviour::new(&body)
        };
        let (url, ranges) = ranged_server(body.clone(), behaviour);
        let client = crate::Client::new("test").unwrap();
        assert_reads(
            &body,
            1_500_000,
            read_at_then_all(start(&client, url), 1_500_000, 1000),
        );
        let ranges: Vec<_> = ranges.try_iter().take(2).collect();
        assert_eq!(
            ranges[0], ranges[1],
            "the failed part was not asked for again"
        );
    }

    #[test]
    fn a_file_changed_mid_download_is_a_read_error() {
        let body = song_body();
        let behaviour = Behaviour {
            total: body.len() + 1,
            ..Behaviour::new(&body)
        };
        let (url, _ranges) = ranged_server(body, behaviour);
        let client = crate::Client::new("test").unwrap();
        let mut download = start(&client, url);
        download.seek(SeekFrom::Start(1_500_000)).unwrap();
        let result = read_in_background(download);
        assert!(result.recv_timeout(WAIT).unwrap().is_err());
    }

    #[test]
    fn an_empty_body_is_read_in_order() {
        let body = Arc::new(Vec::new());
        let behaviour = Behaviour {
            resume_after: Some(Duration::ZERO),
            ..Behaviour::new(&body)
        };
        let (url, _ranges) = ranged_server(body, behaviour);
        let client = crate::Client::new("test").unwrap();
        let download = start(&client, url);
        assert!(!download.is_random_access());
        assert!(
            futures::executor::block_on(download.into_body())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn versions_differ_only_by_headers_both_sent() {
        let mut first = HeaderMap::new();
        first.insert(
            header::LAST_MODIFIED,
            header::HeaderValue::from_static("Tue, 03 Nov 2026 00:00:00 GMT"),
        );
        let version = Version::of(&first);
        assert!(!version.differs(&HeaderMap::new()));
        assert!(!version.differs(&first));
        let mut later = HeaderMap::new();
        later.insert(
            header::LAST_MODIFIED,
            header::HeaderValue::from_static("Wed, 04 Nov 2026 00:00:00 GMT"),
        );
        assert!(version.differs(&later));
    }

    #[test]
    fn a_whole_body_is_handed_over_once_it_arrives() {
        use futures::FutureExt as _;

        let body = song_body();
        // The first response stalls after its head, so the body arrives
        // only once a read far ahead asks for the rest.
        let (url, _ranges) = ranged_server(body.clone(), Behaviour::new(&body));
        let client = crate::Client::new("test").unwrap();
        let download = start(&client, url);
        // Waiting from before the body arrives, as the song cache does.
        let mut completed = Box::pin(download.handle().completed());
        assert!((&mut completed).now_or_never().is_none());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            sender.send(futures::executor::block_on(completed)).ok();
        });
        assert_reads(
            &body,
            1_500_000,
            read_at_then_all(download, 1_500_000, 1000),
        );
        let completed = receiver.recv_timeout(WAIT).expect("still waiting");
        assert!(completed.is_some_and(|completed| completed == body[..]));
    }

    #[test]
    fn a_canceled_download_hands_over_nothing() {
        let (url, _closed) = stalling_server();
        let client = crate::Client::new("test").unwrap();
        let download = start(&client, url);
        let handle = download.handle();
        drop(download);
        assert!(futures::executor::block_on(handle.completed()).is_none());
    }

    #[test]
    fn a_body_of_unknown_length_is_not_handed_over() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nConnection: close\r\n\r\nabc",
                )
                .unwrap();
        });
        let client = crate::Client::new("test").unwrap();
        let download = start(&client, url);
        let handle = download.handle();
        assert_eq!(
            futures::executor::block_on(download.into_body()).unwrap(),
            &b"abc"[..]
        );
        assert!(futures::executor::block_on(handle.completed()).is_none());
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
