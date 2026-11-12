//! Downloads against an in-process HTTP server (`tokio::net::TcpListener`) that answers `Range` requests, a fake
//! stream source and temp folders. Nothing here touches the network or the real app data folder.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use md3_model::TrackItem;
use sf_player::engine::youtube::{BoxFuture, StreamSource, YtStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/youtube/tone_44k_stereo.m4a");
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sf-downloads-{name}-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What the server does for a path.
#[derive(Clone)]
enum Mode {
    /// answers ranges of these bytes
    Serve(Vec<u8>),
    /// always this status
    Status(u16),
    /// this status for the requests with these numbers (1 — the first), the bytes for the others
    FailOn(u16, Vec<usize>, Vec<u8>),
}

#[derive(Default)]
struct ServerState {
    modes: HashMap<String, Mode>,
    /// (path, Range header)
    requests: Vec<(String, String)>,
    /// requests per path, for `FailOn`
    seen: HashMap<String, usize>,
}

struct Server {
    base: String,
    state: Arc<Mutex<ServerState>>,
    /// audio requests being answered now, and the most at once
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

impl Server {
    /// `delay` holds each answer back (to see parallel downloads and cancel one mid-way).
    async fn start(delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state: Arc<Mutex<ServerState>> = Arc::default();
        let (active, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (shared, act, pk) = (state.clone(), active.clone(), peak.clone());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (state, active, peak) = (shared.clone(), act.clone(), pk.clone());
                tokio::spawn(async move {
                    let Some((path, range)) = read_request(&mut socket).await else { return };
                    let mode = {
                        let mut state = state.lock().unwrap();
                        state.requests.push((path.clone(), range.clone()));
                        let seen = state.seen.entry(path.clone()).or_default();
                        *seen += 1;
                        let seen = *seen;
                        match state.modes.get(&path).cloned() {
                            Some(Mode::FailOn(status, numbers, _)) if numbers.contains(&seen) => Mode::Status(status),
                            Some(Mode::FailOn(_, _, body)) => Mode::Serve(body),
                            Some(mode) => mode,
                            None => Mode::Status(404),
                        }
                    };
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(delay).await;
                    let (status, extra, body) = answer(&mode, &range);
                    active.fetch_sub(1, Ordering::SeqCst);
                    let head = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: audio/mp4\r\ncontent-length: {}\r\n{extra}connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self { base, state, active, peak }
    }

    fn set(&self, path: &str, mode: Mode) {
        self.state.lock().unwrap().modes.insert(path.to_owned(), mode);
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn requests(&self, path: &str) -> Vec<String> {
        self.state.lock().unwrap().requests.iter().filter(|(p, _)| p == path).map(|(_, range)| range.clone()).collect()
    }
}

/// (status, extra headers, body)
fn answer(mode: &Mode, range: &str) -> (u16, String, Vec<u8>) {
    let bytes = match mode {
        Mode::Status(status) => return (*status, String::new(), Vec::new()),
        Mode::Serve(bytes) | Mode::FailOn(_, _, bytes) => bytes,
    };
    let Some((start, end)) = range.strip_prefix("bytes=").and_then(|range| range.split_once('-')) else {
        return (200, String::new(), bytes.clone());
    };
    let start: usize = start.parse().unwrap();
    if start >= bytes.len() {
        return (416, format!("content-range: bytes */{}\r\n", bytes.len()), Vec::new());
    }
    let end = end.parse::<usize>().map_or(bytes.len() - 1, |end| end.min(bytes.len() - 1));
    (206, format!("content-range: bytes {start}-{end}/{}\r\n", bytes.len()), bytes[start..=end].to_vec())
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<(String, String)> {
    let mut data = Vec::new();
    let mut buffer = [0u8; 4096];
    while !data.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = socket.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&buffer[..read]);
    }
    let text = String::from_utf8_lossy(&data).into_owned();
    let mut lines = text.split("\r\n");
    let path = lines.next()?.split(' ').nth(1)?.to_owned();
    let range = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("range"))
        .map(|(_, value)| value.trim().to_owned())
        .unwrap_or_default();
    Some((path, range))
}

/// Gives each video the next URL of its list (the last one stays).
#[derive(Default)]
struct FakeSource {
    urls: Mutex<HashMap<String, VecDeque<String>>>,
    resolves: AtomicUsize,
    invalidated: Mutex<Vec<String>>,
    /// content length the streams name
    length: Option<u64>,
}

impl FakeSource {
    fn with(urls: &[(&str, &[String])], length: Option<u64>) -> Arc<Self> {
        let urls = urls.iter().map(|(id, list)| (id.to_string(), list.iter().cloned().collect())).collect();
        Arc::new(Self { urls: Mutex::new(urls), length, ..Default::default() })
    }
}

impl StreamSource for FakeSource {
    fn resolve(&self, video_id: String) -> BoxFuture<Result<YtStream, String>> {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        let mut urls = self.urls.lock().unwrap();
        let url = urls.get_mut(&video_id).and_then(|list| if list.len() > 1 { list.pop_front() } else { list.front().cloned() });
        let length = self.length;
        Box::pin(async move {
            let url = url.ok_or_else(|| "no such song".to_owned())?;
            Ok(YtStream {
                video_id,
                url,
                mime: "audio/mp4".into(),
                codecs: "mp4a.40.2".into(),
                bitrate: 128_000,
                content_length: length,
                user_agent: "test-agent".into(),
                source: "FAKE".into(),
                ..Default::default()
            })
        })
    }

    fn invalidate(&self, video_id: &str) {
        self.invalidated.lock().unwrap().push(video_id.to_owned());
    }
}

fn song(id: &str, cover: &str) -> TrackItem {
    TrackItem {
        title: format!("Song {id}"),
        artists: "Artist".into(),
        album: "Album".into(),
        cover: cover.to_owned(),
        duration_ms: 2_000,
        available: true,
        uri: format!("youtube:track:{id}"),
        ..Default::default()
    }
}

struct Setup {
    downloads: Arc<DownloadsService>,
    folder: PathBuf,
    index: PathBuf,
}

fn setup(source: Arc<FakeSource>, limit_bytes: u64) -> Setup {
    let dir = temp_dir("svc");
    let folder = dir.join("files");
    let index = dir.join("downloads.json");
    let tuning = Tuning { retry_delay: Duration::from_millis(5), chunk_bytes: 7_000 };
    let downloads = DownloadsService::with_tuning(
        Handle::current(),
        sf_api::Http::new(reqwest::Client::new()),
        source,
        index.clone(),
        DownloadsConfig { folder: folder.clone(), limit_bytes },
        tuning,
    );
    Setup { downloads, folder, index }
}

/// Wait (up to 10 s) until the snapshot satisfies `done`.
async fn until(downloads: &DownloadsService, done: impl Fn(&DownloadsSnapshot) -> bool) -> DownloadsSnapshot {
    let mut watch = downloads.watch();
    let wait = async {
        loop {
            let snapshot = watch.borrow_and_update().clone();
            if done(&snapshot) {
                return snapshot;
            }
            watch.changed().await.unwrap();
        }
    };
    tokio::time::timeout(Duration::from_secs(10), wait).await.unwrap_or_else(|_| panic!("timed out: {:?}", downloads.snapshot().jobs))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_song_is_saved_in_ranges_with_its_cover_and_plays_offline() {
    let audio = fixture();
    let server = Server::start(Duration::ZERO).await;
    server.set("/audio", Mode::Serve(audio.clone()));
    server.set("/cover.jpg", Mode::Serve(b"jpeg bytes".to_vec()));
    let source = FakeSource::with(&[("vid1", &[server.url("/audio")])], None);
    let s = setup(source.clone(), 0);

    s.downloads.download(song("vid1", &server.url("/cover.jpg")));
    let snapshot = until(&s.downloads, |snapshot| snapshot.tracks.len() == 1 && snapshot.jobs.is_empty()).await;

    let track = &snapshot.tracks[0];
    assert_eq!(std::fs::read(&track.path).unwrap(), audio);
    assert!(track.path.ends_with("vid1.m4a"), "{}", track.path);
    assert_eq!(track.size, audio.len() as u64);
    assert_eq!(snapshot.bytes, audio.len() as u64);
    assert!(track.single);
    assert_eq!(std::fs::read(&track.cover_path).unwrap(), b"jpeg bytes");
    assert!(track.item().cover.starts_with("file://"));
    assert!(snapshot.is_downloaded("youtube:track:vid1"));
    // ranged pieces of 7000 bytes, with the stream's user agent, none past the end
    let ranges = server.requests("/audio");
    assert_eq!(ranges.len(), audio.len().div_ceil(7_000));
    assert_eq!(ranges[0], "bytes=0-6999");
    assert!(!std::fs::read_dir(&s.folder).unwrap().any(|entry| entry.unwrap().path().to_string_lossy().ends_with(".part")));

    // the index survives a restart
    let saved = std::fs::read_to_string(&s.index).unwrap();
    assert!(saved.contains("vid1"));
    let again = DownloadsService::new(
        Handle::current(),
        sf_api::Http::new(reqwest::Client::new()),
        source.clone(),
        s.index.clone(),
        DownloadsConfig { folder: s.folder.clone(), limit_bytes: 0 },
    );
    assert!(again.is_downloaded("youtube:track:vid1"));

    // the lane's source answers with the file without asking the network source, and the file decodes
    let resolves = source.resolves.load(Ordering::SeqCst);
    let offline = OfflineSource::new(again.clone(), source.clone());
    let stream = offline.resolve_track(StreamRequest::from_track(&song("vid1", ""))).await.unwrap();
    assert_eq!(source.resolves.load(Ordering::SeqCst), resolves);
    assert_eq!(stream.url, format!("{LOCAL_FILE_URL}{}", track.path));
    assert_eq!(stream.codec_label(), "AAC 128");
    assert!(again.snapshot().tracks[0].last_played > 0);
    let file = sf_player::engine::youtube::player::open_local(&stream).unwrap().unwrap();
    let mut decoder = sf_player::engine::youtube::decode::Decoder::open(file, &stream.mime).unwrap();
    let mut frames = 0;
    while let Some(chunk) = decoder.next_chunk().unwrap() {
        frames += chunk.len() / 2;
    }
    assert!((86_000..=92_000).contains(&frames), "two seconds at 44.1 kHz, got {frames}");
    // a song that isn't downloaded still goes to the network source
    let _ = offline.resolve("other".into()).await;
    assert_eq!(source.resolves.load(Ordering::SeqCst), resolves + 1);

    // removing deletes the files
    again.remove("youtube:track:vid1");
    assert!(!std::path::Path::new(&track.path).exists());
    assert!(!std::path::Path::new(&track.cover_path).exists());
    assert!(again.snapshot().tracks.is_empty());
    assert!(again.offline_stream("vid1").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_address_is_resolved_again() {
    let audio = fixture();
    let server = Server::start(Duration::ZERO).await;
    server.set("/old", Mode::Status(403));
    server.set("/new", Mode::Serve(audio.clone()));
    let source = FakeSource::with(&[("vid", &[server.url("/old"), server.url("/new")])], Some(audio.len() as u64));
    let s = setup(source.clone(), 0);

    s.downloads.download(song("vid", ""));
    let snapshot = until(&s.downloads, |snapshot| snapshot.tracks.len() == 1).await;
    assert_eq!(std::fs::read(&snapshot.tracks[0].path).unwrap(), audio);
    assert_eq!(*source.invalidated.lock().unwrap(), ["vid"]);
    assert_eq!(source.resolves.load(Ordering::SeqCst), 2);
    assert_eq!(snapshot.tracks[0].cover_path, "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_failures_are_retried_from_where_the_file_ends() {
    let audio = fixture();
    let server = Server::start(Duration::ZERO).await;
    // the first piece arrives, then two failures, then the rest
    server.set("/flaky", Mode::FailOn(500, vec![2, 3], audio.clone()));
    let source = FakeSource::with(&[("vid", &[server.url("/flaky")])], Some(audio.len() as u64));
    let s = setup(source.clone(), 0);

    s.downloads.download(song("vid", ""));
    let snapshot = until(&s.downloads, |snapshot| snapshot.tracks.len() == 1 || snapshot.jobs.iter().any(|job| matches!(job.state, JobState::Failed(_)))).await;
    assert_eq!(snapshot.tracks.len(), 1, "{:?}", snapshot.jobs);
    assert_eq!(std::fs::read(&snapshot.tracks[0].path).unwrap(), audio);
    let ranges = server.requests("/flaky");
    // no piece after the first one starts again from 0
    assert_eq!(ranges.iter().filter(|range| range.starts_with("bytes=0-")).count(), 1, "{ranges:?}");
    assert_eq!(source.resolves.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_download_that_keeps_failing_can_be_retried() {
    let audio = fixture();
    let server = Server::start(Duration::ZERO).await;
    server.set("/down", Mode::Status(500));
    let source = FakeSource::with(&[("vid", &[server.url("/down")])], None);
    let s = setup(source, 0);

    s.downloads.download(song("vid", ""));
    let snapshot = until(&s.downloads, |snapshot| snapshot.jobs.iter().any(|job| matches!(job.state, JobState::Failed(_)))).await;
    assert_eq!(server.requests("/down").len(), ATTEMPTS as usize);
    assert_eq!(snapshot.jobs[0].attempt, ATTEMPTS);
    assert!(!snapshot.is_pending("youtube:track:vid"));
    assert!(std::fs::read_dir(&s.folder).unwrap().next().is_none(), "the partial file is deleted");

    server.set("/down", Mode::Serve(audio.clone()));
    s.downloads.retry("youtube:track:vid");
    let snapshot = until(&s.downloads, |snapshot| snapshot.tracks.len() == 1).await;
    assert!(snapshot.jobs.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_songs_download_at_a_time_and_a_playlist_is_kept() {
    let audio = fixture();
    let server = Server::start(Duration::from_millis(30)).await;
    let ids = ["a", "b", "c", "d"];
    for id in ids {
        server.set(&format!("/{id}"), Mode::Serve(audio.clone()));
    }
    let urls: Vec<Vec<String>> = ids.iter().map(|id| vec![server.url(&format!("/{id}"))]).collect();
    let pairs: Vec<(&str, &[String])> = ids.iter().zip(&urls).map(|(id, list)| (*id, list.as_slice())).collect();
    let source = FakeSource::with(&pairs, Some(audio.len() as u64));
    let s = setup(source, 0);

    // "a" on its own first; the playlist has a, b, c, d and a Spotify song (skipped)
    s.downloads.download(song("a", ""));
    let mut tracks: Vec<TrackItem> = ids.iter().map(|id| song(id, "")).collect();
    tracks.push(TrackItem { uri: "spotify:track:x".into(), ..Default::default() });
    s.downloads.download_playlist("youtube:playlist:PL1", "Mix", "", tracks);
    let snapshot = s.downloads.snapshot();
    assert_eq!(snapshot.jobs.len(), 4);
    assert_eq!(snapshot.jobs.iter().filter(|job| job.state == JobState::Running).count(), WORKERS);
    assert_eq!(snapshot.playlist_progress("youtube:playlist:PL1"), (0, 4));

    let snapshot = until(&s.downloads, |snapshot| snapshot.tracks.len() == 4).await;
    assert_eq!(server.peak.load(Ordering::SeqCst), WORKERS);
    assert_eq!(server.active.load(Ordering::SeqCst), 0);
    assert_eq!(snapshot.playlist_progress("youtube:playlist:PL1"), (4, 4));
    let order: Vec<String> = s.downloads.playlist_items("youtube:playlist:PL1").into_iter().map(|item| item.uri).collect();
    assert_eq!(order, ids.map(|id| format!("youtube:track:{id}")));

    // removing the playlist keeps the song downloaded on its own
    s.downloads.remove_playlist("youtube:playlist:PL1");
    let snapshot = s.downloads.snapshot();
    assert!(!snapshot.has_playlist("youtube:playlist:PL1"));
    let left: Vec<&str> = snapshot.tracks.iter().map(|track| track.video_id.as_str()).collect();
    assert_eq!(left, ["a"]);
    assert_eq!(std::fs::read_dir(&s.folder).unwrap().filter(|entry| entry.as_ref().unwrap().path().is_file()).count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_download_leaves_nothing() {
    let audio = fixture();
    let server = Server::start(Duration::from_millis(40)).await;
    server.set("/slow", Mode::Serve(audio.clone()));
    let source = FakeSource::with(&[("vid", &[server.url("/slow")])], Some(audio.len() as u64));
    let s = setup(source, 0);

    s.downloads.download(song("vid", ""));
    until(&s.downloads, |snapshot| snapshot.jobs.first().is_some_and(|job| job.downloaded > 0)).await;
    s.downloads.cancel("youtube:track:vid");
    assert!(s.downloads.snapshot().jobs.is_empty());
    tokio::time::sleep(Duration::from_millis(150)).await;
    let snapshot = s.downloads.snapshot();
    assert!(snapshot.tracks.is_empty() && snapshot.jobs.is_empty());
    assert!(std::fs::read_dir(&s.folder).unwrap().next().is_none(), "no partial file");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_storage_limit_refuses_new_downloads() {
    let audio = fixture();
    let server = Server::start(Duration::ZERO).await;
    server.set("/a", Mode::Serve(audio.clone()));
    let source = FakeSource::with(&[("a", &[server.url("/a")])], Some(audio.len() as u64));
    let s = setup(source, (audio.len() / 2) as u64);

    s.downloads.download(song("a", ""));
    let snapshot = until(&s.downloads, |snapshot| snapshot.jobs.iter().any(|job| matches!(job.state, JobState::Failed(_)))).await;
    assert_eq!(snapshot.jobs[0].state, JobState::Failed(mf_i18n::t("Downloads have reached their storage limit")));
    assert!(server.requests("/a").is_empty());
    assert!(snapshot.tracks.is_empty());

    s.downloads.set_limit(0);
    s.downloads.retry_failed();
    until(&s.downloads, |snapshot| snapshot.tracks.len() == 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn songs_not_played_for_long_are_cleaned_up() {
    let audio = fixture();
    let server = Server::start(Duration::ZERO).await;
    server.set("/a", Mode::Serve(audio.clone()));
    server.set("/b", Mode::Serve(audio.clone()));
    let source = FakeSource::with(&[("a", &[server.url("/a")]), ("b", &[server.url("/b")])], None);
    let s = setup(source, 0);
    s.downloads.download(song("a", ""));
    s.downloads.download(song("b", ""));
    until(&s.downloads, |snapshot| snapshot.tracks.len() == 2).await;

    // "a" was downloaded 100 days ago and never played; "b" was played just now
    {
        let mut inner = s.downloads.lock();
        for track in inner.tracks.iter_mut() {
            track.downloaded_at -= 100 * 86_400;
        }
    }
    s.downloads.mark_played("b");
    let forgotten: Vec<String> = s.downloads.forgotten(30).into_iter().map(|track| track.video_id).collect();
    assert_eq!(forgotten, ["a"]);
    assert_eq!(s.downloads.remove_forgotten(30), (1, audio.len() as u64));
    let left: Vec<String> = s.downloads.snapshot().tracks.iter().map(|track| track.video_id.clone()).collect();
    assert_eq!(left, ["b"]);
}

#[test]
fn file_names_and_settings() {
    assert_eq!(file_stem("dQw4w9WgXcQ"), "dQw4w9WgXcQ");
    assert_eq!(file_stem("../x"), "___x");
    let flac = YtStream { mime: "audio/flac".into(), ..Default::default() };
    assert_eq!(extension(&flac), "flac");
    assert_eq!(extension(&YtStream { mime: "audio/mp4; codecs=\"mp4a.40.2\"".into(), ..Default::default() }), "m4a");
    assert_eq!(limit_from_setting(-5), 0);
    assert_eq!(limit_from_setting(2), 2 * 1024 * 1024);
    assert_eq!(folder_from_setting("  D:/Music  "), PathBuf::from("D:/Music"));
    assert!(folder_from_setting("").ends_with("downloads"));
    let job = DownloadJob { uri: String::new(), title: String::new(), artists: String::new(), state: JobState::Queued, downloaded: 5, total: Some(10), attempt: 1 };
    assert_eq!(job.progress(), 0.5);
}
