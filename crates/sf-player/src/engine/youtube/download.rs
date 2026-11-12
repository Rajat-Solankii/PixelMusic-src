//! Progressive download of a stream URL into memory: a tokio task asks for `Range: bytes=a-b` chunks one after
//! another and appends them to a shared buffer; [`StreamReader`] reads from that buffer (`Read + Seek`, and a
//! symphonia `MediaSource`), blocking until the bytes at its position arrive. After a seek far from what has
//! arrived the download continues from there (a FLAC of tens of megabytes would otherwise stay silent until
//! everything before the target is in) and comes back for what it skipped. Dropping the reader stops the download.
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// How much one request asks for. YouTube throttles a whole-file request, not 4 MiB pieces.
pub const CHUNK_BYTES: u64 = 4 * 1024 * 1024;
/// A reader waiting within a quarter of a chunk ahead of the download lets it arrive; farther, the download jumps.
const NEAR_PART: u64 = 4;
/// The first request after a jump asks for a sixteenth of a chunk.
const JUMP_PART: u64 = 16;
/// A reader that has read this part of a chunk on from where it last seeked is playing, not probing.
const PLAYING_PART: u64 = 64;
/// How long a read waits for bytes before it gives up.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Further attempts at a chunk after a network failure or an unexpected status.
const RETRIES: u32 = 3;
/// How long an answer may keep back its head or its next bytes. A server that stops sending in the middle of an
/// answer (Monochrome's does at times) is asked again from where it stopped instead of keeping the song silent.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(8);

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

/// One downloaded stretch of the file.
struct Piece {
    start: u64,
    data: Vec<u8>,
}

impl Piece {
    fn end(&self) -> u64 {
        self.start + self.data.len() as u64
    }
}

#[derive(Default)]
struct State {
    /// what has arrived: sorted by start, never overlapping or touching
    pieces: Vec<Piece>,
    /// where the download writes next
    head: u64,
    /// where the reader waits for bytes the download is not about to reach (a seek): the download goes there
    want: Option<u64>,
    /// a reader this close ahead of `head` just waits for the download
    near: u64,
    /// [`PLAYING_PART`] of a chunk
    playing: u64,
    /// total size, once a response or the stream description names it
    total: Option<u64>,
    done: bool,
    error: Option<DownloadError>,
    cancelled: bool,
}

impl State {
    fn piece_at(&self, position: u64) -> Option<&Piece> {
        self.pieces.iter().find(|piece| piece.start <= position && position < piece.end())
    }

    fn covers(&self, position: u64) -> bool {
        self.piece_at(position).is_some()
    }

    /// The first offset at or after `from` that has not arrived.
    fn gap_from(&self, from: u64) -> u64 {
        self.piece_at(from).map_or(from, Piece::end)
    }

    /// Where the piece after `position` begins.
    fn next_start(&self, position: u64) -> Option<u64> {
        self.pieces.iter().map(|piece| piece.start).find(|&start| start > position)
    }

    /// Bytes from the start of the file without a gap.
    fn prefix(&self) -> u64 {
        self.pieces.first().filter(|piece| piece.start == 0).map_or(0, Piece::end)
    }

    fn downloaded(&self) -> u64 {
        self.pieces.iter().map(|piece| piece.data.len() as u64).sum()
    }

    /// Stores `bytes` at `position`; what is already there stays.
    fn put(&mut self, mut position: u64, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            if let Some(piece) = self.piece_at(position) {
                let skip = ((piece.end() - position) as usize).min(bytes.len());
                position += skip as u64;
                bytes = &bytes[skip..];
                continue;
            }
            let room = self.next_start(position).map_or(bytes.len() as u64, |next| next - position);
            let take = (room as usize).min(bytes.len());
            let index = match self.pieces.iter().position(|piece| piece.end() == position) {
                Some(index) => {
                    self.pieces[index].data.extend_from_slice(&bytes[..take]);
                    index
                }
                None => {
                    let index = self.pieces.iter().position(|piece| piece.start > position).unwrap_or(self.pieces.len());
                    self.pieces.insert(index, Piece { start: position, data: bytes[..take].to_vec() });
                    index
                }
            };
            position += take as u64;
            bytes = &bytes[take..];
            // it reached the next piece: one piece from now on
            if self.pieces.get(index + 1).is_some_and(|next| next.start == position) {
                let next = self.pieces.remove(index + 1);
                self.pieces[index].data.extend_from_slice(&next.data);
            }
        }
    }
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
}

/// What to download.
#[derive(Clone, Debug)]
pub struct DownloadSpec {
    pub url: String,
    /// sent as `User-Agent` (the URL is tied to the client that asked for it); empty — the client's own
    pub user_agent: String,
    pub content_length: Option<u64>,
    pub chunk_bytes: u64,
    /// [`IDLE_TIMEOUT`]
    pub idle_timeout: Duration,
}

impl DownloadSpec {
    pub fn new(url: impl Into<String>, user_agent: impl Into<String>, content_length: Option<u64>) -> Self {
        Self {
            url: url.into(),
            user_agent: user_agent.into(),
            content_length,
            chunk_bytes: CHUNK_BYTES,
            idle_timeout: IDLE_TIMEOUT,
        }
    }
}

/// Starts the download on `runtime` and returns a reader over it.
pub fn start(client: reqwest::Client, runtime: &tokio::runtime::Handle, spec: DownloadSpec) -> StreamReader {
    let shared = Arc::new(Shared::default());
    {
        let mut state = shared.lock();
        state.total = spec.content_length.filter(|&length| length > 0);
        state.near = (spec.chunk_bytes / NEAR_PART).max(1);
        state.playing = (spec.chunk_bytes / PLAYING_PART).max(1);
    }
    let task = runtime.spawn(run(client, spec, shared.clone()));
    StreamReader {
        shared: shared.clone(),
        position: 0,
        since: 0,
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
    // the next request starts where the reader jumped to: a short one, so its first bytes are not behind a long answer
    let mut jumped = false;
    loop {
        let range = {
            let mut state = shared.lock();
            if state.cancelled {
                return;
            }
            // without the size there is no telling a seek target from the end of the file
            if state.total.is_some()
                && let Some(want) = state.want.take()
                && !state.covers(want)
            {
                state.head = want;
                jumped = true;
            }
            let mut head = state.gap_from(state.head);
            if let Some(total) = state.total
                && head >= total
            {
                // the end is here; what a jump skipped comes next
                head = state.gap_from(0);
            }
            state.head = head;
            if state.total.is_some_and(|total| head >= total) {
                None
            } else {
                let mut end = head + if jumped { (chunk / JUMP_PART).max(1) } else { chunk } - 1;
                if let Some(total) = state.total {
                    end = end.min(total - 1);
                }
                if let Some(next) = state.next_start(head) {
                    end = end.min(next - 1);
                }
                Some((head, end))
            }
        };
        let Some((offset, end)) = range else {
            shared.update(|state| state.done = true);
            return;
        };
        match fetch(&client, &spec, offset, end, &shared).await {
            Ok(Fetched::Bytes(0)) => {
                failures += 1;
                if failures > RETRIES {
                    shared.update(|state| state.error = Some(DownloadError::Failed("empty response".into())));
                    return;
                }
            }
            Ok(Fetched::Bytes(_)) => {
                failures = 0;
                jumped = false;
            }
            Ok(Fetched::Interrupted) => failures = 0,
            Ok(Fetched::End(length)) => {
                let length = length.unwrap_or_else(|| shared.lock().prefix());
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
    /// this many bytes arrived
    Bytes(u64),
    /// the reader jumped elsewhere while the answer was arriving
    Interrupted,
    /// the file ended: its length when the whole file came in this answer, `None` when the server says there is
    /// nothing at this offset
    End(Option<u64>),
}

async fn fetch(
    client: &reqwest::Client,
    spec: &DownloadSpec,
    offset: u64,
    end: u64,
    shared: &Shared,
) -> Result<Fetched, DownloadError> {
    let mut request = client.get(&spec.url).header(reqwest::header::RANGE, format!("bytes={offset}-{end}"));
    if !spec.user_agent.is_empty() {
        request = request.header(reqwest::header::USER_AGENT, &spec.user_agent);
    }
    let failed = |error: reqwest::Error| DownloadError::Failed(error.without_url().to_string());
    let idle = spec.idle_timeout;
    let silent = || DownloadError::Failed(format!("nothing arrived for {idle:?}"));
    let mut response = tokio::time::timeout(idle, request.send()).await.map_err(|_| silent())?.map_err(failed)?;
    let status = response.status().as_u16();
    // A range past the end once the size is unknown means the file is complete.
    if status == 416 && offset > 0 {
        return Ok(Fetched::End(None));
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
    // 200: the server ignored the range and sends the whole file from its start.
    let ranged = status == 206;
    let mut cursor = if ranged { offset } else { 0 };
    let mut arrived = 0u64;
    loop {
        let bytes = match tokio::time::timeout(idle, response.chunk()).await {
            Ok(Ok(Some(bytes))) => bytes,
            Ok(Ok(None)) => break,
            Ok(Err(error)) if arrived == 0 => return Err(failed(error)),
            Err(_) if arrived == 0 => return Err(silent()),
            // cut off or stalled after some bytes: they are kept, and the next request goes on from there
            Ok(Err(_)) | Err(_) => {
                log::debug!("youtube download: the answer at {offset} stopped after {arrived} bytes; asking again");
                return Ok(Fetched::Bytes(arrived));
            }
        };
        if bytes.is_empty() {
            continue;
        }
        arrived += bytes.len() as u64;
        let mut state = shared.lock();
        if state.cancelled {
            return Ok(Fetched::Bytes(arrived));
        }
        state.put(cursor, &bytes);
        cursor += bytes.len() as u64;
        state.head = cursor;
        let elsewhere = ranged && state.total.is_some() && state.want.is_some_and(|want| !state.covers(want));
        drop(state);
        shared.arrived.notify_all();
        if elsewhere {
            return Ok(Fetched::Interrupted);
        }
    }
    if !ranged {
        // the whole file came in one answer
        return Ok(Fetched::End(Some(cursor)));
    }
    Ok(Fetched::Bytes(arrived))
}

/// The total of `bytes 0-99/1234` (`*` — unknown).
fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse().ok()
}

/// Reads the downloaded bytes; blocks until the bytes at its position arrive.
pub struct StreamReader {
    shared: Arc<Shared>,
    position: u64,
    /// where the reader last seeked to: reading on from there is playback (the read-ahead below)
    since: u64,
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
        self.0.lock().downloaded()
    }

    pub fn total(&self) -> Option<u64> {
        self.0.lock().total
    }

    pub fn finished(&self) -> bool {
        self.0.lock().done
    }

    /// Where what has arrived from `position` on ends.
    #[cfg(test)]
    fn arrived_until(&self, position: u64) -> u64 {
        self.0.lock().gap_from(position)
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
        {
            let mut state = self.shared.lock();
            let past_end = state.total.is_some_and(|total| position >= total);
            if !state.done && !past_end {
                if !state.covers(position) {
                    // after a seek the bytes may be far from where the download is: it comes here instead of
                    // the reader waiting for everything in between
                    let near = position >= state.head && position - state.head <= state.near;
                    if !near {
                        state.want = Some(position);
                    }
                } else if position.saturating_sub(self.since) >= state.playing {
                    // playing a stretch fetched for another position (a FLAC seek probes the file here and there
                    // and lands in one of them): the download goes on right after it, not where it last was. Not
                    // while probing: each move of the download costs a round trip.
                    let gap = state.gap_from(position);
                    if gap != state.head && state.total.is_some_and(|total| gap < total) && state.want != Some(gap) {
                        state.want = Some(gap);
                    }
                }
            }
        }
        let state = self.wait(|state| state.covers(position) || state.total.is_some_and(|total| position >= total))?;
        let Some(piece) = state.piece_at(position) else {
            // at or past the end of a finished file
            return Ok(0);
        };
        let start = (position - piece.start) as usize;
        let count = buf.len().min(piece.data.len() - start);
        buf[..count].copy_from_slice(&piece.data[start..start + count]);
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
                let total = state.total.unwrap_or(state.prefix());
                Some(total as i128 + delta as i128)
            }
        };
        match target {
            Some(target) if target >= 0 => {
                if target as u64 != self.position {
                    self.since = target as u64;
                }
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
        self.wait(|state| state.total.is_some()).ok().and_then(|state| state.total.or(Some(state.prefix())))
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
        /// ranges answered after a delay of `n` ms
        Slow(u64),
        /// each range answer sends at most `n` bytes, then holds the connection without sending more
        Stalls(usize),
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
                    if let Mode::Slow(ms) = mode {
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                    }
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
                    if let (Mode::Stalls(most), Some(_)) = (mode, range) {
                        // the head and the first `most` bytes of the body, then nothing
                        let header = response.windows(4).position(|window| window == b"\r\n\r\n").map_or(0, |at| at + 4);
                        let _ = socket.write_all(&response[..(header + most).min(response.len())]).await;
                        if header + most < response.len() {
                            tokio::time::sleep(Duration::from_secs(3600)).await;
                        }
                        return;
                    }
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
    async fn a_far_seek_is_served_before_the_bytes_in_between() {
        let body = sample_body(4_000_000);
        let (url, requests) = serve(body.clone(), Mode::Ranges).await;
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec(&url, 16_000));
        let status = reader.status();
        let expected = body.clone();
        let mut reader = tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 1_000];
            reader.read_exact(&mut buf).unwrap();
            assert_eq!(&buf[..], &expected[..1_000]);
            reader.seek(SeekFrom::Start(3_500_000)).unwrap();
            reader.read_exact(&mut buf).unwrap();
            assert_eq!(&buf[..], &expected[3_500_000..3_501_000]);
            reader
        })
        .await
        .unwrap();
        // 4 MB in 16 kB requests is 250 of them; the target came long before that
        assert!(requests.load(Ordering::SeqCst) < 60, "{} requests", requests.load(Ordering::SeqCst));
        // what the jump skipped still arrives, and the file reads whole
        let read = tokio::task::spawn_blocking(move || {
            reader.seek(SeekFrom::Start(0)).unwrap();
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert!(read == body);
        assert_eq!(status.downloaded(), 4_000_000);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reading_an_earlier_stretch_brings_the_download_back_after_it() {
        // every answer takes a while, as a far server's do
        let body = sample_body(4_000_000);
        let (url, _) = serve(body.clone(), Mode::Slow(150)).await;
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec(&url, 64_000));
        let status = reader.status();
        let end = tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 100];
            // probes far apart, as a FLAC seek makes them, then it lands in the first one
            for at in [3_000_000, 1_000_000, 3_000_000] {
                reader.seek(SeekFrom::Start(at)).unwrap();
                reader.read_exact(&mut buf).unwrap();
            }
            // then it plays on (past a 64th of a chunk): that, not the probing, brings the download back
            for _ in 0..20 {
                reader.read_exact(&mut buf).unwrap();
            }
            (reader, status.arrived_until(3_000_000))
        })
        .await
        .unwrap();
        let (reader, end) = end;
        let status = reader.status();
        // the download left 1 MB for what follows the stretch being read, before the reader got there
        // (going on from 1 MB in 64 kB answers would take seconds)
        let started = std::time::Instant::now();
        while status.arrived_until(3_000_000) <= end + 64_000 {
            assert!(started.elapsed() < Duration::from_millis(1_500), "the download stayed elsewhere");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(reader);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_answer_that_stops_sending_is_asked_again_from_where_it_stopped() {
        let body = sample_body(200_000);
        let (url, requests) = serve(body.clone(), Mode::Stalls(30_000)).await;
        let spec = DownloadSpec { idle_timeout: Duration::from_millis(100), ..spec(&url, 64_000) };
        let mut reader = start(client(), &tokio::runtime::Handle::current(), spec);
        let status = reader.status();
        let read = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert!(read == body);
        assert_eq!(status.error(), None);
        // 30 kB an answer: seven of them, none counted as a failure
        assert_eq!(requests.load(Ordering::SeqCst), 7);
    }

    #[test]
    fn pieces_merge_and_keep_what_they_have() {
        let mut state = State::default();
        state.put(10, &[1, 2, 3]);
        state.put(0, &[9; 4]);
        assert_eq!((state.pieces.len(), state.prefix(), state.gap_from(11), state.next_start(4)), (2, 4, 13, Some(10)));
        // fills the gap, runs into the second piece and past it
        state.put(4, &[7; 12]);
        assert_eq!(state.pieces.len(), 1);
        assert_eq!(state.pieces[0].data, [9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 1, 2, 3, 7, 7, 7]);
        assert_eq!((state.prefix(), state.downloaded(), state.gap_from(0)), (16, 16, 16));
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
