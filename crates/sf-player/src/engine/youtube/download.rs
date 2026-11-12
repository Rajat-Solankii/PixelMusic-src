//! Progressive download of a stream URL into memory: a tokio task asks for `Range: bytes=a-b` chunks one after
//! another and appends them to a shared buffer; [`StreamReader`] reads from that buffer (`Read + Seek`, and a
//! symphonia `MediaSource`), blocking until the bytes at its position arrive. A seek past what has arrived
//! waits for the sequential download to get there. Dropping the reader stops the download.
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// How much one request asks for. YouTube throttles a whole-file request, not 4 MiB pieces.
pub const CHUNK_BYTES: u64 = 4 * 1024 * 1024;
/// How long a read waits for bytes before it gives up.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Further attempts at a chunk after a network failure or an unexpected status.
const RETRIES: u32 = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a download stopped before the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadError {
    /// 403 / 410: the URL expired or was refused; resolve the stream again.
    Expired,
    /// the answer is not the stream: a page or error text, or another size than the stream's; resolve again
    Invalid(String),
    /// network failure or unexpected answer, after the retries
    Failed(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expired => f.write_str("the stream address expired"),
            Self::Invalid(what) => write!(f, "the stream address answered without audio ({what})"),
            Self::Failed(error) => write!(f, "download failed: {error}"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl DownloadError {
    /// What the listener is told when the song could not be opened because of this.
    pub fn user_message(&self) -> String {
        match self {
            Self::Expired | Self::Invalid(_) => mf_i18n::t("YouTube didn't send this song's audio. Try again in a moment."),
            Self::Failed(_) => mf_i18n::t("The song's audio couldn't be downloaded — check your connection."),
        }
    }
}

/// A content type that cannot be a stream: pages, JSON or XML error bodies, manifests.
fn is_not_media(content_type: &str) -> bool {
    let kind = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    kind.starts_with("text/") || kind.contains("json") || kind.contains("html") || kind.contains("xml")
}

#[derive(Default)]
struct State {
    data: Vec<u8>,
    /// total size, once a response or the stream description names it
    total: Option<u64>,
    done: bool,
    error: Option<DownloadError>,
    cancelled: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    arrived: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.lock());
        self.arrived.notify_all();
    }

    fn cancelled(&self) -> bool {
        self.lock().cancelled
    }
}

/// What to download.
#[derive(Clone, Debug)]
pub struct DownloadSpec {
    pub url: String,
    /// sent as `User-Agent` (the URL is tied to the client that asked for it); empty — the client's own
    pub user_agent: String,
    pub content_length: Option<u64>,
    pub chunk_bytes: u64,
}

impl DownloadSpec {
    pub fn new(url: impl Into<String>, user_agent: impl Into<String>, content_length: Option<u64>) -> Self {
        Self { url: url.into(), user_agent: user_agent.into(), content_length, chunk_bytes: CHUNK_BYTES }
    }
}

/// Starts the download on `runtime` and returns a reader over it.
pub fn start(client: reqwest::Client, runtime: &tokio::runtime::Handle, spec: DownloadSpec) -> StreamReader {
    let shared = Arc::new(Shared::default());
    shared.lock().total = spec.content_length.filter(|&length| length > 0);
    let task = runtime.spawn(run(client, spec, shared.clone()));
    StreamReader {
        shared: shared.clone(),
        position: 0,
        timeout: READ_TIMEOUT,
        _guard: Arc::new(Guard { shared, task: task.abort_handle() }),
    }
}

/// Stops the download when the last reader holding it goes away.
struct Guard {
    shared: Arc<Shared>,
    task: tokio::task::AbortHandle,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.shared.update(|state| state.cancelled = true);
        self.task.abort();
    }
}

async fn run(client: reqwest::Client, spec: DownloadSpec, shared: Arc<Shared>) {
    let chunk = spec.chunk_bytes.max(1);
    let mut failures = 0u32;
    loop {
        if shared.cancelled() {
            return;
        }
        let (offset, total) = {
            let state = shared.lock();
            (state.data.len() as u64, state.total)
        };
        if total.is_some_and(|total| offset >= total) {
            shared.update(|state| state.done = true);
            return;
        }
        let mut end = offset + chunk - 1;
        if let Some(total) = total {
            end = end.min(total - 1);
        }
        match fetch(&client, &spec, offset, end, &shared).await {
            Ok(Fetched::Bytes(0)) => {
                failures += 1;
                if failures > RETRIES {
                    shared.update(|state| state.error = Some(DownloadError::Failed("empty response".into())));
                    return;
                }
            }
            Ok(Fetched::Bytes(_)) => failures = 0,
            Ok(Fetched::End) => {
                let length = shared.lock().data.len() as u64;
                if let Some(expected) = spec.content_length.filter(|&expected| expected > 0 && expected != length) {
                    // a short answer with a success status: not the stream
                    let error = DownloadError::Invalid(format!("{length} bytes instead of {expected}"));
                    log::info!("youtube download: {error}");
                    shared.update(|state| state.error = Some(error));
                    return;
                }
                shared.update(|state| {
                    state.done = true;
                    state.total = Some(length);
                });
                return;
            }
            Err(error @ (DownloadError::Expired | DownloadError::Invalid(_))) => {
                if let DownloadError::Invalid(_) = &error {
                    log::info!("youtube download: {error}");
                }
                shared.update(|state| state.error = Some(error));
                return;
            }
            Err(error) => {
                failures += 1;
                if failures > RETRIES {
                    log::warn!("youtube download: {error}");
                    shared.update(|state| state.error = Some(error));
                    return;
                }
                tokio::time::sleep(Duration::from_millis(400 << failures)).await;
            }
        }
    }
}

enum Fetched {
    /// this many bytes were appended
    Bytes(u64),
    /// the server says there is nothing at this offset: the file ended
    End,
}

async fn fetch(
    client: &reqwest::Client,
    spec: &DownloadSpec,
    offset: u64,
    end: u64,
    shared: &Shared,
) -> Result<Fetched, DownloadError> {
    let mut request = client
        .get(&spec.url)
        .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
        .timeout(REQUEST_TIMEOUT);
    if !spec.user_agent.is_empty() {
        request = request.header(reqwest::header::USER_AGENT, &spec.user_agent);
    }
    let failed = |error: reqwest::Error| DownloadError::Failed(error.without_url().to_string());
    let mut response = request.send().await.map_err(failed)?;
    let status = response.status().as_u16();
    // A range past the end once the size is unknown means the file is complete.
    if status == 416 && offset > 0 {
        return Ok(Fetched::End);
    }
    if status == 403 || status == 410 {
        return Err(DownloadError::Expired);
    }
    if !matches!(status, 200 | 206) {
        return Err(DownloadError::Failed(format!("response {status}")));
    }
    if let Some(kind) = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
        && is_not_media(kind)
    {
        return Err(DownloadError::Invalid(format!("{status} {kind}")));
    }
    // the file's size as the server sees it: from Content-Range, or the whole answer's length
    let total = if status == 206 {
        response.headers().get(reqwest::header::CONTENT_RANGE).and_then(|value| value.to_str().ok()).and_then(content_range_total)
    } else {
        response.content_length().filter(|&length| length > 0)
    };
    if let (Some(expected), Some(total)) = (spec.content_length.filter(|&expected| expected > 0), total)
        && expected != total
    {
        return Err(DownloadError::Invalid(format!("{total} bytes instead of {expected}")));
    }
    if let Some(total) = total {
        shared.update(|state| state.total = Some(total));
    }
    // 200: the server ignored the range and sends the whole file; skip what is already here.
    let mut skip = if status == 206 { 0 } else { offset };
    let mut appended = 0u64;
    while let Some(bytes) = response.chunk().await.map_err(failed)? {
        if shared.cancelled() {
            return Ok(Fetched::Bytes(appended));
        }
        let mut bytes = &bytes[..];
        if skip > 0 {
            let dropped = skip.min(bytes.len() as u64);
            skip -= dropped;
            bytes = &bytes[dropped as usize..];
        }
        if bytes.is_empty() {
            continue;
        }
        appended += bytes.len() as u64;
        shared.update(|state| state.data.extend_from_slice(bytes));
    }
    if status == 200 {
        // the whole file came in one answer
        return Ok(Fetched::End);
    }
    Ok(Fetched::Bytes(appended))
}

/// The total of `bytes 0-99/1234` (`*` — unknown).
fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse().ok()
}

/// Reads the downloaded bytes; blocks until the bytes at its position arrive.
pub struct StreamReader {
    shared: Arc<Shared>,
    position: u64,
    timeout: Duration,
    _guard: Arc<Guard>,
}

/// A view of a download's progress that outlives handing the reader to the decoder.
#[derive(Clone)]
pub struct DownloadStatus(Arc<Shared>);

impl DownloadStatus {
    /// Why the download stopped early, if it did.
    pub fn error(&self) -> Option<DownloadError> {
        self.0.lock().error.clone()
    }

    pub fn downloaded(&self) -> u64 {
        self.0.lock().data.len() as u64
    }

    pub fn total(&self) -> Option<u64> {
        self.0.lock().total
    }

    pub fn finished(&self) -> bool {
        self.0.lock().done
    }
}

impl StreamReader {
    pub fn status(&self) -> DownloadStatus {
        DownloadStatus(self.shared.clone())
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Waits until `ready` holds for the state, the download fails or the timeout passes.
    fn wait<'a>(&'a self, mut ready: impl FnMut(&State) -> bool) -> io::Result<MutexGuard<'a, State>> {
        let deadline = std::time::Instant::now() + self.timeout;
        let mut state = self.shared.lock();
        loop {
            if ready(&state) || state.done {
                return Ok(state);
            }
            if let Some(error) = &state.error {
                let kind = match error {
                    DownloadError::Expired => io::ErrorKind::PermissionDenied,
                    DownloadError::Invalid(_) => io::ErrorKind::InvalidData,
                    DownloadError::Failed(_) => io::ErrorKind::Other,
                };
                return Err(io::Error::new(kind, error.clone()));
            }
            if state.cancelled {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "download cancelled"));
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "no audio data arrived in time"));
            }
            state = self.shared.arrived.wait_timeout(state, left).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
        }
    }
}

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let position = self.position;
        let state = self.wait(|state| {
            state.data.len() as u64 > position || state.total.is_some_and(|total| position >= total)
        })?;
        let available = state.data.len() as u64;
        if position >= available {
            // at or past the end of a finished file
            return Ok(0);
        }
        let start = position as usize;
        let count = buf.len().min(available as usize - start);
        buf[..count].copy_from_slice(&state.data[start..start + count]);
        drop(state);
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => Some(offset as i128),
            SeekFrom::Current(delta) => Some(self.position as i128 + delta as i128),
            SeekFrom::End(delta) => {
                let state = self.wait(|state| state.total.is_some())?;
                let total = state.total.unwrap_or(state.data.len() as u64);
                Some(total as i128 + delta as i128)
            }
        };
        match target {
            Some(target) if target >= 0 => {
                self.position = target as u64;
                Ok(self.position)
            }
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before the start of the stream")),
        }
    }
}

impl symphonia::core::io::MediaSource for StreamReader {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        // the MP4 reader needs the size; the first response names it quickly
        self.wait(|state| state.total.is_some()).ok().and_then(|state| state.total.or(Some(state.data.len() as u64)))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// How the test server answers.
    #[derive(Clone, Copy, PartialEq)]
    pub(crate) enum Mode {
        Ranges,
        Forbidden,
        /// first `n` requests fail with 500
        FailFirst(usize),
        /// 200 with a small HTML page, whatever was asked
        Html,
        /// 206 answers that say the file is `n` bytes long
        ClaimsTotal(usize),
    }

    /// An HTTP server on 127.0.0.1 serving `body` at any path, honouring `Range`. Returns the URL and a count of
    /// requests.
    pub(crate) async fn serve(body: Vec<u8>, mode: Mode) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = Arc::new(body);
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let body = body.clone();
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if socket.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let index = counter.fetch_add(1, Ordering::SeqCst);
                    let head = String::from_utf8_lossy(&head).to_lowercase();
                    let range = head.lines().find_map(|line| line.strip_prefix("range: bytes=")).map(|range| {
                        let (start, end) = range.trim().split_once('-').unwrap();
                        let start: usize = start.parse().unwrap();
                        let end: usize = end.parse().unwrap_or(body.len() - 1);
                        (start, end.min(body.len().saturating_sub(1)))
                    });
                    let response = match (mode, range) {
                        (Mode::Forbidden, _) => b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
                        (Mode::Html, _) => {
                            let page = "<html><body>Sorry, something went wrong.</body></html>";
                            format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}", page.len())
                                .into_bytes()
                        }
                        (Mode::ClaimsTotal(claimed), Some((start, end))) => {
                            let mut out = format!(
                                "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {start}-{end}/{claimed}\r\nconnection: close\r\n\r\n",
                                end - start + 1
                            )
                            .into_bytes();
                            out.extend_from_slice(&body[start..=end]);
                            out
                        }
                        (Mode::FailFirst(count), _) if index < count => {
                            b"HTTP/1.1 500 Oops\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec()
                        }
                        (_, Some((start, _))) if start >= body.len() => {
                            b"HTTP/1.1 416 Range Not Satisfiable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec()
                        }
                        (_, Some((start, end))) => {
                            let mut out = format!(
                                "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {start}-{end}/{}\r\nconnection: close\r\n\r\n",
                                end - start + 1,
                                body.len()
                            )
                            .into_bytes();
                            out.extend_from_slice(&body[start..=end]);
                            out
                        }
                        (_, None) => {
                            let mut out = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len())
                                .into_bytes();
                            out.extend_from_slice(&body);
                            out
                        }
                    };
                    let _ = socket.write_all(&response).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (format!("http://{address}/videoplayback"), requests)
    }

    pub(crate) fn sample_body(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 7 % 251) as u8).collect()
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn spec(url: &str, chunk: u64) -> DownloadSpec {
        DownloadSpec { chunk_bytes: chunk, ..DownloadSpec::new(url, "test-agent", None) }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reads_the_whole_file_in_chunks() {
        let body = sample_body(100_000);
        let (url, requests) = serve(body.clone(), Mode::Ranges).await;
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec(&url, 16_384));
        let status = reader.status();
        let read = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert_eq!(read, body);
        assert_eq!(status.total(), Some(100_000));
        // (the reader is gone by now, so the task may have been cancelled before marking itself done)
        assert_eq!(status.downloaded(), 100_000);
        // 100 000 / 16 384 → 7 chunks
        assert_eq!(requests.load(Ordering::SeqCst), 7);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn seeks_forward_back_and_from_the_end() {
        let body = sample_body(50_000);
        let (url, _) = serve(body.clone(), Mode::Ranges).await;
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec(&url, 8_000));
        tokio::task::spawn_blocking(move || {
            use symphonia::core::io::MediaSource;
            assert_eq!(reader.byte_len(), Some(50_000));
            let mut buf = [0u8; 100];
            reader.seek(SeekFrom::Start(40_000)).unwrap();
            reader.read_exact(&mut buf).unwrap();
            assert_eq!(&buf[..], &body[40_000..40_100]);
            reader.seek(SeekFrom::Start(10)).unwrap();
            reader.read_exact(&mut buf).unwrap();
            assert_eq!(&buf[..], &body[10..110]);
            assert_eq!(reader.seek(SeekFrom::End(-50)).unwrap(), 49_950);
            let mut tail = Vec::new();
            reader.read_to_end(&mut tail).unwrap();
            assert_eq!(&tail[..], &body[49_950..]);
            assert!(reader.seek(SeekFrom::Current(-100_000)).is_err());
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forbidden_means_expired() {
        let (url, requests) = serve(sample_body(1_000), Mode::Forbidden).await;
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec(&url, 4_096));
        let status = reader.status();
        let error = tokio::task::spawn_blocking(move || reader.read(&mut [0u8; 16]).unwrap_err()).await.unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(status.error(), Some(DownloadError::Expired));
        // no retry for an expired address
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_errors_are_retried() {
        let body = sample_body(3_000);
        let (url, _) = serve(body.clone(), Mode::FailFirst(2)).await;
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec(&url, 4_096));
        let read = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert_eq!(read, body);
    }

    /// Reads until the download fails: the read error and the download's own.
    async fn failure(url: &str, expected: Option<u64>) -> (io::Error, Option<DownloadError>) {
        let spec = DownloadSpec { chunk_bytes: 4_096, ..DownloadSpec::new(url, "test-agent", expected) };
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec);
        let status = reader.status();
        let error = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap_err()
        })
        .await
        .unwrap();
        (error, status.error())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_page_instead_of_audio_is_invalid_and_not_retried() {
        let (url, requests) = serve(sample_body(1_000), Mode::Html).await;
        let (error, status) = failure(&url, Some(5_818_745)).await;
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(matches!(&status, Some(DownloadError::Invalid(what)) if what.contains("text/html")), "{status:?}");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        // without a known size too
        let (_, status) = failure(&url, None).await;
        assert!(matches!(status, Some(DownloadError::Invalid(_))));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_short_answer_with_a_success_status_is_invalid() {
        // the file is 1372 bytes where the format said almost 6 MB
        let (url, _) = serve(sample_body(1_372), Mode::Ranges).await;
        let (error, status) = failure(&url, Some(5_818_745)).await;
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(status, Some(DownloadError::Invalid("1372 bytes instead of 5818745".into())));
        // a range answer that names another size
        let (url, _) = serve(sample_body(1_372), Mode::ClaimsTotal(9_999)).await;
        let (_, status) = failure(&url, Some(1_372)).await;
        assert_eq!(status, Some(DownloadError::Invalid("9999 bytes instead of 1372".into())));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_expected_length_is_accepted() {
        let body = sample_body(10_000);
        let (url, _) = serve(body.clone(), Mode::Ranges).await;
        let spec = DownloadSpec { chunk_bytes: 4_096, ..DownloadSpec::new(&url, "test-agent", Some(10_000)) };
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec);
        let read = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert_eq!(read, body);
    }

    #[test]
    fn content_types_that_are_not_audio() {
        assert!(is_not_media("text/html; charset=utf-8"));
        assert!(is_not_media("application/json"));
        assert!(is_not_media("application/dash+xml"));
        assert!(!is_not_media("audio/mp4"));
        assert!(!is_not_media("application/octet-stream"));
        assert!(!is_not_media("audio/flac"));
    }

    #[test]
    fn content_range_totals() {
        assert_eq!(content_range_total("bytes 0-99/1234"), Some(1234));
        assert_eq!(content_range_total("bytes 0-99/*"), None);
    }
}
