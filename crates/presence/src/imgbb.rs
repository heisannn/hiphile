use std::time::{Duration, SystemTime};

use bytes::Bytes;
use catalog::{CoverLink, CoverLinks};
use futures::lock::Mutex;
use net::http;
use serde::Deserialize;
use url::Url;

/// Where files are uploaded to: <https://api.imgbb.com/>.
const API: &str = "https://api.imgbb.com/1/upload";
/// When ImgBB is asked to delete an upload, in seconds: 180 days, the longest
/// it allows.
const KEEP_FOR: &str = "15552000";
/// How long a link is used: less than [`KEEP_FOR`], so that it does not stop
/// working under a song just started.
const USE_FOR: Duration = Duration::from_secs(179 * 24 * 60 * 60);
/// The longest image URL Discord accepts.
const LONGEST_URL: usize = 300;

/// Says nothing of the API key, so it can be shown.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the server sent something other than an image")]
    NotAnImage,
    #[error(transparent)]
    Links(#[from] catalog::Error),
    #[error(transparent)]
    Net(#[from] net::Error),
    #[error("ImgBB refused it: {0}")]
    Refused(String),
    #[error("ImgBB answered with something other than a link")]
    UnexpectedAnswer,
}

type OpenLinks = Box<dyn Fn() -> Result<CoverLinks, catalog::Error> + Send + Sync>;

/// Publishes cover art on ImgBB for a limited time, for Discord to show.
///
/// Discord needs a public URL for an image, and the one on the user's server
/// carries their credentials, so the image itself is uploaded instead. Anyone
/// with the link can see an upload until it expires. An image is uploaded again
/// only once its link has run out, and without the record of links, not at all.
pub struct CoverHost {
    net: net::Client,
    api: Url,
    open_links: OpenLinks,
    /// Opened on first use, off the UI thread. Being a lock around uploading
    /// too, it keeps an image that is asked for twice from going up twice.
    links: Mutex<Option<CoverLinks>>,
}

impl CoverHost {
    pub fn imgbb(net: net::Client) -> Self {
        Self::new(
            net,
            Url::parse(API).expect("the API's address is a valid URL"),
            Box::new(CoverLinks::user),
        )
    }

    fn new(net: net::Client, api: Url, open_links: OpenLinks) -> Self {
        Self {
            net,
            api,
            open_links,
            links: Mutex::new(None),
        }
    }

    /// A public URL of `image`, uploading it with `key` unless one still works.
    /// Blocks on the disk: await it where blocking is allowed.
    pub async fn link(&self, image: &[u8], key: &str) -> Result<CoverLink, Error> {
        let kind = ImageKind::of(image).ok_or(Error::NotAnImage)?;
        let mut links = self.links.lock().await;
        let links = match &mut *links {
            Some(links) => links,
            empty => empty.insert((self.open_links)()?),
        };
        let now = SystemTime::now();
        if let Some(link) = links.get(image, now)? {
            return Ok(link);
        }
        let link = CoverLink {
            url: self.upload(image, kind, key).await?,
            expires: now + USE_FOR,
        };
        // A failure to record it costs an upload later.
        links.put(image, &link).ok();
        Ok(link)
    }

    async fn upload(&self, image: &[u8], kind: ImageKind, key: &str) -> Result<String, Error> {
        let mut url = self.api.clone();
        url.query_pairs_mut()
            .append_pair("key", key)
            .append_pair("expiration", KEEP_FOR);
        let boundary = boundary_for(image);
        let request = http::Request::post(url.as_str())
            .header(
                http::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(upload_body(image, kind, &boundary))
            .expect("a parsed Url and a valid boundary make a valid request");
        let response = self.net.send(request).await?;
        let answer = serde_json::from_slice::<Answer>(response.body()).ok();
        if let Some(message) = answer
            .as_ref()
            .and_then(|answer| answer.error.as_ref())
            .map(|error| error.message.as_str())
        {
            return Err(Error::Refused(message.replace(key, "…")));
        }
        if !response.status().is_success() {
            return Err(net::Error::HttpStatus(response.status().as_u16()).into());
        }
        answer
            .and_then(|answer| answer.data)
            .and_then(|data| usable_link(&data.url))
            .ok_or(Error::UnexpectedAnswer)
    }
}

#[derive(Deserialize)]
struct Answer {
    data: Option<AnswerData>,
    error: Option<AnswerError>,
}

#[derive(Deserialize)]
struct AnswerData {
    url: String,
}

#[derive(Deserialize)]
struct AnswerError {
    message: String,
}

/// The file's name says what kind of image it is and nothing about the album.
fn upload_body(image: &[u8], kind: ImageKind, boundary: &str) -> Bytes {
    let mut body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"image\"; filename=\"cover.{}\"\r\n\
         Content-Type: {}\r\n\r\n",
        kind.extension(),
        kind.mime_type(),
    )
    .into_bytes();
    body.extend_from_slice(image);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body.into()
}

/// A multipart boundary that does not occur in `image`.
fn boundary_for(image: &[u8]) -> String {
    (0u32..)
        .map(|attempt| format!("hiphile-cover-{attempt}"))
        .find(|boundary| {
            !image
                .windows(boundary.len())
                .any(|window| window == boundary.as_bytes())
        })
        .expect("an image cannot hold every number")
}

/// `link` if it is on ImgBB, over HTTPS, and short enough for Discord.
fn usable_link(link: &str) -> Option<String> {
    let url = Url::parse(link).ok()?;
    let host = url.host_str()?;
    let on_imgbb = host == "ibb.co" || host.ends_with(".ibb.co");
    (url.scheme() == "https" && on_imgbb && link.len() <= LONGEST_URL).then(|| link.to_owned())
}

/// Told from the image's first bytes. A server reports a failure as a document
/// with a success status, which must not get published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageKind {
    Jpeg,
    Png,
    Gif,
    Webp,
}

impl ImageKind {
    fn of(image: &[u8]) -> Option<Self> {
        match image {
            [0xff, 0xd8, 0xff, ..] => Some(Self::Jpeg),
            [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some(Self::Png),
            [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some(Self::Gif),
            [
                b'R',
                b'I',
                b'F',
                b'F',
                _,
                _,
                _,
                _,
                b'W',
                b'E',
                b'B',
                b'P',
                ..,
            ] => Some(Self::Webp),
            _ => None,
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }

    fn mime_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::thread;

    use futures::executor::block_on;

    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe rest of the image";
    const KEY: &str = "the-api-key";
    const LINK: &str = "https://i.ibb.co/abc123/cover.png";
    const UPLOADED: &str = r#"{"data":{"id":"abc123","url":"https://i.ibb.co/abc123/cover.png","expiration":"15552000"},"success":true,"status":200}"#;

    /// Answers every request with `status` and `answer`, keeping the requests.
    fn fake_imgbb(status: u16, answer: &'static str) -> (Url, Arc<StdMutex<Vec<Vec<u8>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}/1/upload", listener.local_addr().unwrap());
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let kept = requests.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let request = read_request(&mut stream);
                kept.lock().unwrap().push(request);
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (Url::parse(&api).unwrap(), requests)
    }

    fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            let end = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|at| at + 4);
            if let Some(end) = end {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .map_or(0, |length| length.trim().parse().unwrap());
                if request.len() >= end + length {
                    return request;
                }
            }
            let read = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..read]);
        }
    }

    fn host(api: Url, dir: PathBuf) -> CoverHost {
        let net = net::Client::new("hiphile-test").unwrap();
        CoverHost::new(net, api, Box::new(move || CoverLinks::open(&dir)))
    }

    #[test]
    fn an_image_is_uploaded_for_a_limited_time_and_its_link_returned() {
        let dir = tempfile::tempdir().unwrap();
        let (api, requests) = fake_imgbb(200, UPLOADED);

        let before = SystemTime::now();
        let link = block_on(host(api, dir.path().to_owned()).link(PNG, KEY)).unwrap();

        assert_eq!(link.url, LINK);
        assert!(link.is_live(before + USE_FOR - Duration::from_secs(1)));
        assert!(!link.is_live(before + USE_FOR + Duration::from_secs(60)));
        let requests = requests.lock().unwrap();
        let request = String::from_utf8_lossy(&requests[0]);
        assert!(
            request.starts_with("POST /1/upload?key=the-api-key&expiration=15552000 "),
            "{request}"
        );
        assert!(request.contains("name=\"image\"; filename=\"cover.png\""));
        assert!(request.contains("Content-Type: image/png\r\n\r\n"));
        let body = &requests[0];
        assert!(body.windows(PNG.len()).any(|window| window == PNG));
    }

    #[test]
    fn an_image_goes_up_once_also_across_launches() {
        let dir = tempfile::tempdir().unwrap();
        let (api, requests) = fake_imgbb(200, UPLOADED);

        let first = host(api.clone(), dir.path().to_owned());
        assert_eq!(block_on(first.link(PNG, KEY)).unwrap().url, LINK);
        assert_eq!(block_on(first.link(PNG, KEY)).unwrap().url, LINK);
        let next_launch = host(api, dir.path().to_owned());
        assert_eq!(block_on(next_launch.link(PNG, KEY)).unwrap().url, LINK);

        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn an_image_goes_up_again_once_its_link_has_run_out() {
        let dir = tempfile::tempdir().unwrap();
        let expired = CoverLink {
            url: "https://i.ibb.co/old/cover.png".to_owned(),
            expires: SystemTime::now() - Duration::from_secs(1),
        };
        CoverLinks::open(dir.path())
            .unwrap()
            .put(PNG, &expired)
            .unwrap();
        let (api, requests) = fake_imgbb(200, UPLOADED);

        let link = block_on(host(api, dir.path().to_owned()).link(PNG, KEY)).unwrap();

        assert_eq!(link.url, LINK);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_failure_document_is_never_uploaded() {
        let dir = tempfile::tempdir().unwrap();
        let (api, requests) = fake_imgbb(200, UPLOADED);
        let document = br#"<subsonic-response status="failed"/>"#;

        let result = block_on(host(api, dir.path().to_owned()).link(document, KEY));

        assert!(matches!(result, Err(Error::NotAnImage)), "{result:?}");
        assert!(requests.lock().unwrap().is_empty());
    }

    #[test]
    fn without_the_record_of_uploads_nothing_is_uploaded() {
        let (api, requests) = fake_imgbb(200, UPLOADED);
        let net = net::Client::new("hiphile-test").unwrap();
        let host = CoverHost::new(net, api, Box::new(|| Err(catalog::Error::NoDataDir)));

        let result = block_on(host.link(PNG, KEY));

        assert!(matches!(result, Err(Error::Links(_))), "{result:?}");
        assert!(requests.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refusal_says_why_and_never_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _) = fake_imgbb(
            400,
            r#"{"status_code":400,"error":{"message":"Invalid API v1 key the-api-key.","code":100},"status_txt":"Bad Request"}"#,
        );

        let error = block_on(host(api, dir.path().to_owned()).link(PNG, KEY)).unwrap_err();

        let shown = error.to_string();
        assert!(shown.contains("Invalid API v1 key"), "{shown}");
        assert!(!shown.contains(KEY), "{shown}");
    }

    #[test]
    fn a_network_failure_never_says_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}/1/upload", listener.local_addr().unwrap());
        drop(listener);

        let error = block_on(host(Url::parse(&api).unwrap(), dir.path().to_owned()).link(PNG, KEY))
            .unwrap_err();

        assert!(!format!("{error} {error:?}").contains(KEY), "{error:?}");
    }

    #[test]
    fn a_failed_upload_keeps_no_link_and_is_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        for (status, answer) in [
            (500, "Internal error"),
            (200, "not json"),
            (
                200,
                r#"{"data":{"url":"https://elsewhere.example.com/a.png"}}"#,
            ),
            (200, r#"{"data":{"url":"http://i.ibb.co/a.png"}}"#),
        ] {
            let (api, _) = fake_imgbb(status, answer);
            let host = host(api, dir.path().to_owned());
            assert!(block_on(host.link(PNG, KEY)).is_err(), "{answer}");
        }

        let (api, requests) = fake_imgbb(200, UPLOADED);
        let link = block_on(host(api, dir.path().to_owned()).link(PNG, KEY)).unwrap();
        assert_eq!(link.url, LINK);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_boundary_is_not_in_the_image() {
        assert_eq!(boundary_for(PNG), "hiphile-cover-0");
        assert_eq!(boundary_for(b"..hiphile-cover-0.."), "hiphile-cover-1");
    }

    #[test]
    fn images_are_told_by_their_first_bytes() {
        assert_eq!(ImageKind::of(b"\xff\xd8\xff\xe0"), Some(ImageKind::Jpeg));
        assert_eq!(ImageKind::of(PNG), Some(ImageKind::Png));
        assert_eq!(ImageKind::of(b"GIF89a.."), Some(ImageKind::Gif));
        assert_eq!(ImageKind::of(b"RIFF\0\0\0\0WEBPVP8"), Some(ImageKind::Webp));
        assert_eq!(ImageKind::of(b"RIFF\0\0\0\0WAVEfmt"), None);
        assert_eq!(ImageKind::of(b""), None);
    }
}
