//! Monochrome, JioSaavn and the chain against an in-process HTTP server (`tokio::net::TcpListener`) and recorded
//! or hand-written answers in `tests/fixtures/lossless`. Nothing here talks to the real servers.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use sf_player::engine::youtube::{BoxFuture, StreamRequest, StreamSource, YtStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::matching::Candidate;
use super::monochrome::{self, Monochrome};
use super::saavn::{self, Saavn, SaavnQuality, SaavnSong};
use super::{Cooldown, SourceChain, SourceError};

const KESARIYA_URL: &str = "http://aac.saavncdn.com/430/5c5ea5cc00e3bff45616013226f376fe_96.mp4";
/// `KESARIYA_URL` through Java's `DES/ECB/PKCS5Padding` with key `38346591`, as the API sends it.
const KESARIYA_ENCRYPTED: &str =
    "iPPGVzyogeiPwpro65A0eUaQggN+8+J4soHGLpOWVT6K6RkNB6mF8AT9ZQU9MUmbaWcMz14qsUwlaFcDXhieroPzFaL/aK97";
const SHORT_URL: &str = "http://aac.saavncdn.com/123/0a1b2c3d4e5f_96.mp4";

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lossless").join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn fixture(name: &str) -> Value {
    serde_json::from_slice(&fixture_bytes(name)).unwrap()
}

#[derive(Clone, Debug)]
struct Request {
    path: String,
    query: HashMap<String, String>,
    /// lower-case names
    headers: HashMap<String, String>,
}

/// status, content type, body
type Reply = (u16, &'static str, Vec<u8>);

fn json(status: u16, body: &Value) -> Reply {
    (status, "application/json", body.to_string().into_bytes())
}

struct MockServer {
    base: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl MockServer {
    async fn start(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let handler = Arc::new(handler);
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (handler, recorded) = (handler.clone(), recorded.clone());
                tokio::spawn(async move {
                    let Some(request) = read_request(&mut socket).await else { return };
                    recorded.lock().unwrap().push(request.clone());
                    let (status, content_type, body) = handler(&request);
                    let head = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self { base, requests }
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    fn count(&self, path: &str) -> usize {
        self.requests().iter().filter(|request| request.path.ends_with(path)).count()
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<Request> {
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
    let target = lines.next()?.split(' ').nth(1)?.to_owned();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .collect();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let query = url_decode_pairs(query);
    Some(Request { path: path.to_owned(), query, headers })
}

fn url_decode_pairs(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |text: &str| percent_decode(&text.replace('+', " "));
            (decode(name), decode(value))
        })
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = text.get(index + 1..index + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let (b'%', Some(byte)) = (bytes[index], hex) {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn http() -> sf_api::Http {
    sf_api::Http::new(reqwest::Client::builder().no_proxy().build().expect("client"))
}

fn queen() -> StreamRequest {
    StreamRequest {
        video_id: "fJ9rUzIMcZQ".into(),
        title: "Bohemian Rhapsody".into(),
        artists: vec!["Queen".into()],
        album: "A Night at the Opera".into(),
        duration_ms: 354_000,
        explicit: false,
    }
}

fn kesariya() -> StreamRequest {
    StreamRequest {
        video_id: "BddP6PYo2gs".into(),
        title: "Kesariya (Official Video)".into(),
        artists: vec!["Arijit Singh".into()],
        album: "Brahmastra".into(),
        duration_ms: 268_400,
        explicit: false,
    }
}

// ─── Monochrome ────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn monochrome_search_answers_are_parsed() {
    let tracks = monochrome::parse_search(&fixture("monochrome_search.json"));
    let ids: Vec<&str> = tracks.iter().map(|track| track.id.as_str()).collect();
    // unplayable and id-less hits are dropped; numeric ids and `trackId` are read
    assert_eq!(ids, ["111", "222", "444"]);
    assert_eq!(
        tracks[2].candidate,
        Candidate {
            title: "Bohemian Rhapsody - Remastered 2011".into(),
            artist: Some("Queen".into()),
            album: Some("A Night At The Opera (2011 Remaster)".into()),
            duration_ms: Some(354_947),
        }
    );
    // seconds become milliseconds
    let seconds = monochrome::parse_search(&serde_json::json!({ "tracks": [{ "id": 1, "title": "x", "duration": 200 }] }));
    assert_eq!(seconds[0].candidate.duration_ms, Some(200_000));
    assert!(monochrome::parse_search(&Value::Null).is_empty());
}

#[tokio::test]
async fn monochrome_resolves_the_best_match_to_a_flac_url() {
    let server = MockServer::start(|_| json(200, &fixture("monochrome_search.json"))).await;
    let source = Monochrome::new(http()).with_base_url(&server.base);
    let stream = source.resolve(&queen()).await.unwrap().expect("a match");
    assert_eq!(stream.url, format!("{}/track/444", server.base));
    assert_eq!((stream.mime.as_str(), stream.codecs.as_str()), ("audio/flac", "flac"));
    assert_eq!((stream.sample_rate, stream.bit_depth, stream.bitrate), (44_100, 16, 1_411_200));
    assert_eq!(stream.codec_label(), "FLAC 16/44.1");
    assert_eq!((stream.video_id.as_str(), stream.source.as_str()), ("fJ9rUzIMcZQ", "Monochrome"));
    assert_eq!(stream.duration_ms, 354_947);
    assert_eq!(stream.user_agent, monochrome::USER_AGENT);

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/search/tracks");
    assert_eq!(requests[0].query["q"], "Queen Bohemian Rhapsody");
    assert_eq!(requests[0].query["limit"], "5");
    assert_eq!(requests[0].headers["user-agent"], monochrome::USER_AGENT);
    assert!(source.is_up());
}

#[tokio::test]
async fn monochrome_without_an_accepted_match_gives_nothing() {
    let server = MockServer::start(|_| json(200, &fixture("monochrome_search.json"))).await;
    let source = Monochrome::new(http()).with_base_url(&server.base);
    let request = StreamRequest { title: "Radio Ga Ga".into(), ..queen() };
    assert_eq!(source.resolve(&request).await, Ok(None));
    // no title: not even a search
    assert_eq!(source.resolve(&StreamRequest::video("x")).await, Ok(None));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn monochrome_rests_after_a_server_error() {
    let server = MockServer::start(|_| json(503, &serde_json::json!({ "error": "down" }))).await;
    let source = Monochrome::new(http()).with_base_url(&server.base);
    assert_eq!(source.resolve(&queen()).await, Err(SourceError::Status(503)));
    assert!(!source.is_up());
    // resting: no request
    assert_eq!(source.resolve(&queen()).await, Ok(None));
    assert_eq!(server.requests().len(), 1);
    // a successful check brings it back
    source.cooldown.mark_up();
    assert!(source.is_up());
}

#[tokio::test]
async fn monochrome_rests_when_unreachable_but_not_on_a_404() {
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let source = Monochrome::new(http()).with_base_url(&closed);
    assert!(matches!(source.resolve(&queen()).await, Err(SourceError::Network(_))));
    assert!(!source.is_up());

    let server = MockServer::start(|_| (404, "text/plain", b"no".to_vec())).await;
    let source = Monochrome::new(http()).with_base_url(&server.base);
    assert_eq!(source.check().await, Err(SourceError::Status(404)));
    assert!(source.is_up());
}

#[test]
fn cooldowns_rest_and_recover() {
    let cooldown = Cooldown::new(Duration::from_millis(30));
    assert!(cooldown.is_up());
    cooldown.record::<()>(&Err(SourceError::Status(502)));
    assert!(!cooldown.is_up());
    std::thread::sleep(Duration::from_millis(40));
    assert!(cooldown.is_up());
    cooldown.record::<()>(&Err(SourceError::Parse("x".into())));
    assert!(cooldown.is_up(), "a bad answer is not an outage");
    cooldown.mark_down();
    cooldown.record(&Ok(()));
    assert!(cooldown.is_up());
}

// ─── JioSaavn ──────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn media_urls_decrypt_like_javas_des() {
    assert_eq!(saavn::decrypt_url(KESARIYA_ENCRYPTED), KESARIYA_URL);
    // the MIME decoder of the Kotlin app tolerates line breaks
    let wrapped = format!("{}\n{}", &KESARIYA_ENCRYPTED[..40], &KESARIYA_ENCRYPTED[40..]);
    assert_eq!(saavn::decrypt_url(&wrapped), KESARIYA_URL);
    assert_eq!(saavn::decrypt_url(""), "");
    assert_eq!(saavn::decrypt_url("not base64!"), "");
    assert_eq!(saavn::decrypt_url("AAAAAAAAAAA="), "", "bad padding");
}

#[test]
fn the_quality_is_picked_by_suffix() {
    assert_eq!(
        saavn::quality_url(KESARIYA_URL, SaavnQuality::Kbps320),
        "https://aac.saavncdn.com/430/5c5ea5cc00e3bff45616013226f376fe_320.mp4"
    );
    assert_eq!(saavn::quality_url(SHORT_URL, SaavnQuality::Kbps160), "https://aac.saavncdn.com/123/0a1b2c3d4e5f_160.mp4");
    assert_eq!(saavn::quality_url("https://x/a_48.aac", SaavnQuality::Kbps96), "https://x/a_96.aac");
    // no suffix: the first `_96` is replaced
    assert_eq!(saavn::quality_url("https://x/a_96/b.mp4", SaavnQuality::Kbps320), "https://x/a_320/b.mp4");
}

#[test]
fn titles_are_cleaned() {
    assert_eq!(saavn::clean_title("Kesariya (Official Video)"), "Kesariya");
    assert_eq!(saavn::clean_title("Song [Lyrics] ft. Someone"), "Song");
    assert_eq!(saavn::clean_title("Song (Audio)  feat. X"), "Song");
    assert_eq!(saavn::clean_title("Left Alone"), "Left Alone");
}

#[test]
fn songs_are_parsed() {
    let results = fixture("saavn_search.json");
    let songs: Vec<SaavnSong> = results["results"].as_array().unwrap().iter().filter_map(saavn::parse_song).collect();
    assert_eq!(songs.len(), 4);
    assert!(songs[0].pro_only);
    // a string where a list belongs is no list
    assert_eq!(songs[1].artists, ["Someone Else"]);
    let kesariya = &songs[3];
    assert_eq!(kesariya.title, "Kesariya (From \"Brahmastra\")");
    assert_eq!(kesariya.album, "Kesariya (From \"Brahmastra\")");
    // without a full list: primary, then featured
    assert_eq!(kesariya.artists, ["Pritam", "Arijit Singh", "Amitabh Bhattacharya"]);
    assert_eq!(kesariya.duration, Some(268));
    assert!(!kesariya.explicit && !kesariya.pro_only);
    assert_eq!(kesariya.image, "https://c.saavncdn.com/430/Kesariya-500x500.jpg");
    assert_eq!(kesariya.media_url, KESARIYA_URL);
    assert_eq!(saavn::parse_song(&serde_json::json!({ "title": "no id" })), None);
}

#[test]
fn the_first_fitting_song_wins() {
    let results = fixture("saavn_search.json");
    let songs: Vec<SaavnSong> = results["results"].as_array().unwrap().iter().filter_map(saavn::parse_song).collect();
    // Pro-only, another artist and a 400 s cut are passed over
    assert_eq!(saavn::best_match(&kesariya(), &songs).map(|song| song.id.as_str()), Some("OqVAbcde"));

    let song = |id: &str, album: &str, explicit: bool| SaavnSong {
        id: id.into(),
        title: "Song".into(),
        artists: vec!["Artist".into()],
        album: album.into(),
        duration: Some(200),
        explicit,
        ..Default::default()
    };
    let request = StreamRequest {
        title: "Song".into(),
        artists: vec!["Artist".into()],
        album: "The Album".into(),
        duration_ms: 205_000,
        explicit: true,
        ..Default::default()
    };
    let songs = [song("other", "Other", false), song("clean", "The Album", false), song("explicit", "The Album", true)];
    // album first, then the explicit flag
    assert_eq!(saavn::best_match(&request, &songs).map(|song| song.id.as_str()), Some("explicit"));
    let none = StreamRequest { artists: vec!["Nobody".into()], ..request.clone() };
    assert_eq!(saavn::best_match(&none, &songs), None);
    let far = StreamRequest { duration_ms: 230_000, ..request };
    assert_eq!(saavn::best_match(&far, &songs), None);
}

#[tokio::test]
async fn saavn_falls_back_to_the_query_without_the_album() {
    let server = MockServer::start(|request| {
        if request.query["q"].starts_with("Brahmastra") {
            json(200, &serde_json::json!({ "total": 0, "results": [] }))
        } else {
            json(200, &fixture("saavn_search.json"))
        }
    })
    .await;
    let source = Saavn::new(http()).with_base_url(&format!("{}/api.php", server.base));
    let stream = source.resolve(&kesariya(), SaavnQuality::Kbps320).await.unwrap().expect("a match");
    assert_eq!(stream.url, "https://aac.saavncdn.com/430/5c5ea5cc00e3bff45616013226f376fe_320.mp4");
    assert_eq!((stream.mime.as_str(), stream.codecs.as_str(), stream.bitrate), ("audio/mp4", "mp4a.40.2", 320_000));
    assert_eq!(stream.codec_label(), "AAC 320");
    assert_eq!((stream.source.as_str(), stream.video_id.as_str()), ("JioSaavn", "BddP6PYo2gs"));
    assert_eq!(stream.duration_ms, 268_000);
    assert_eq!(stream.author, "Pritam, Arijit Singh, Amitabh Bhattacharya");
    assert_eq!(stream.thumbnail, "https://c.saavncdn.com/430/Kesariya-500x500.jpg");

    let requests = server.requests();
    let queries: Vec<&str> = requests.iter().map(|request| request.query["q"].as_str()).collect();
    assert_eq!(queries, ["Brahmastra Kesariya Arijit Singh", "Kesariya Arijit Singh"]);
    let first = &requests[0];
    assert_eq!(first.path, "/api.php");
    for (name, value) in
        [("__call", "search.getResults"), ("_format", "json"), ("api_version", "4"), ("ctx", "android"), ("n", "30")]
    {
        assert_eq!(first.query[name], value, "{name}");
    }
    assert_eq!(first.headers["cookie"], "explicit_content=1");
    assert_eq!(first.headers["user-agent"], saavn::USER_AGENT);

    let low = source.resolve(&kesariya(), SaavnQuality::Kbps96).await.unwrap().unwrap();
    assert!(low.url.ends_with("_96.mp4"));
    assert_eq!(low.bitrate, 96_000);
}

#[tokio::test]
async fn saavn_asks_for_details_when_the_hit_has_no_media_url() {
    let server = MockServer::start(|request| match request.query["__call"].as_str() {
        "search.getResults" => {
            let mut song = fixture("saavn_details.json")["nourl"].clone();
            song["more_info"]["encrypted_media_url"] = Value::String(String::new());
            json(200, &serde_json::json!({ "results": [song] }))
        }
        _ => json(200, &fixture("saavn_details.json")),
    })
    .await;
    let source = Saavn::new(http()).with_base_url(&server.base);
    let request = StreamRequest {
        video_id: "v".into(),
        title: "Apna Bana Le".into(),
        artists: vec!["Arijit Singh".into()],
        ..Default::default()
    };
    let stream = source.resolve(&request, SaavnQuality::Kbps320).await.unwrap().expect("a match");
    assert_eq!(stream.url, "https://aac.saavncdn.com/123/0a1b2c3d4e5f_320.mp4");
    let requests = server.requests();
    assert_eq!(requests.len(), 2, "no album: one query, then the details");
    assert_eq!((requests[1].query["__call"].as_str(), requests[1].query["pids"].as_str()), ("song.getDetails", "nourl"));
}

#[tokio::test]
async fn saavn_rests_after_a_server_error() {
    let server = MockServer::start(|_| json(500, &Value::Null)).await;
    let source = Saavn::new(http()).with_base_url(&server.base);
    assert_eq!(source.resolve(&kesariya(), SaavnQuality::Kbps320).await, Err(SourceError::Status(500)));
    assert!(!source.is_up());
    assert_eq!(source.resolve(&kesariya(), SaavnQuality::Kbps320).await, Ok(None));
    assert_eq!(server.requests().len(), 1);
}

// ─── The chain ─────────────────────────────────────────────────────────────────────────────────────────────

/// YouTube as the lane would see it: records what it was asked.
#[derive(Default)]
struct FakeYouTube {
    resolved: Mutex<Vec<String>>,
    prefetched: Mutex<Vec<String>>,
    invalidated: Mutex<Vec<String>>,
}

impl StreamSource for FakeYouTube {
    fn resolve(&self, video_id: String) -> BoxFuture<Result<YtStream, String>> {
        self.resolved.lock().unwrap().push(video_id.clone());
        Box::pin(async move {
            Ok(YtStream {
                url: format!("https://example.invalid/{video_id}"),
                mime: "audio/mp4".into(),
                codecs: "mp4a.40.2".into(),
                source: "ANDROID_VR".into(),
                video_id,
                ..Default::default()
            })
        })
    }

    fn prefetch(&self, video_id: String) -> BoxFuture<()> {
        self.prefetched.lock().unwrap().push(video_id);
        Box::pin(async {})
    }

    fn invalidate(&self, video_id: &str) {
        self.invalidated.lock().unwrap().push(video_id.to_owned());
    }
}

/// Monochrome at `/search/tracks` + `/track/<id>`, JioSaavn at `/api.php`.
async fn catalogs(monochrome_status: u16) -> MockServer {
    MockServer::start(move |request| match request.path.as_str() {
        "/search/tracks" if monochrome_status != 200 => json(monochrome_status, &Value::Null),
        "/search/tracks" => json(200, &fixture("monochrome_search.json")),
        "/track/444" => (200, "audio/flac", fixture_bytes("tone_44k_stereo.flac")),
        "/api.php" => json(200, &fixture("saavn_search.json")),
        _ => (404, "text/plain", Vec::new()),
    })
    .await
}

fn chain(server: &MockServer, youtube: Arc<FakeYouTube>) -> SourceChain {
    SourceChain::with_sources(
        youtube,
        Monochrome::new(http()).with_base_url(&server.base),
        Saavn::new(http()).with_base_url(&format!("{}/api.php", server.base)),
    )
}

#[tokio::test]
async fn with_both_off_youtube_plays() {
    let server = catalogs(200).await;
    let youtube = Arc::new(FakeYouTube::default());
    let chain = chain(&server, youtube.clone());
    let stream = chain.resolve_track(queen()).await.unwrap();
    assert_eq!(stream.source, "ANDROID_VR");
    assert_eq!(*youtube.resolved.lock().unwrap(), ["fJ9rUzIMcZQ"]);
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn monochrome_comes_first_and_is_cached() {
    let server = catalogs(200).await;
    let youtube = Arc::new(FakeYouTube::default());
    let chain = chain(&server, youtube.clone());
    chain.set_lossless(true);
    chain.set_saavn(true);
    let stream = chain.resolve_track(queen()).await.unwrap();
    assert_eq!((stream.source.as_str(), stream.codecs.as_str()), ("Monochrome", "flac"));
    let again = chain.resolve_track(queen()).await.unwrap();
    assert_eq!(again, stream);
    assert_eq!(server.count("/search/tracks"), 1, "the second resolve is cached");
    assert_eq!(server.count("/api.php"), 0);
    assert!(youtube.resolved.lock().unwrap().is_empty());
    // a bare video id (no metadata) still goes to YouTube
    chain.resolve("other".into()).await.unwrap();
    assert_eq!(*youtube.resolved.lock().unwrap(), ["other"]);
}

#[tokio::test]
async fn saavn_follows_when_monochrome_has_nothing() {
    let server = catalogs(200).await;
    let youtube = Arc::new(FakeYouTube::default());
    let chain = chain(&server, youtube.clone());
    chain.set_lossless(true);
    chain.set_saavn(true);
    let stream = chain.resolve_track(kesariya()).await.unwrap();
    assert_eq!(stream.source, "JioSaavn");
    assert!(stream.url.ends_with("_320.mp4"));
    assert_eq!(server.count("/search/tracks"), 1);
    assert!(youtube.resolved.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_data_saver_skips_lossless_and_takes_96k() {
    let server = catalogs(200).await;
    let youtube = Arc::new(FakeYouTube::default());
    let chain = chain(&server, youtube.clone());
    chain.set_lossless(true);
    chain.set_data_saver(true);
    // lossless alone, with the data saver: straight to YouTube
    assert_eq!(chain.resolve_track(queen()).await.unwrap().source, "ANDROID_VR");
    assert!(server.requests().is_empty());
    chain.set_saavn(true);
    let stream = chain.resolve_track(kesariya()).await.unwrap();
    assert!(stream.url.ends_with("_96.mp4"), "{}", stream.url);
    assert_eq!(server.count("/search/tracks"), 0);
}

#[tokio::test]
async fn a_failed_stream_sends_the_track_to_youtube() {
    let server = catalogs(200).await;
    let youtube = Arc::new(FakeYouTube::default());
    let chain = chain(&server, youtube.clone());
    chain.set_lossless(true);
    assert_eq!(chain.resolve_track(queen()).await.unwrap().source, "Monochrome");
    // the player could not open it
    chain.invalidate("fJ9rUzIMcZQ");
    assert_eq!(*youtube.invalidated.lock().unwrap(), ["fJ9rUzIMcZQ"]);
    assert_eq!(chain.resolve_track(queen()).await.unwrap().source, "ANDROID_VR");
    assert_eq!(server.count("/search/tracks"), 1);
    // a settings change forgets that
    chain.clear();
    assert_eq!(chain.resolve_track(queen()).await.unwrap().source, "Monochrome");
}

#[tokio::test]
async fn a_resting_source_is_skipped() {
    let server = catalogs(503).await;
    let youtube = Arc::new(FakeYouTube::default());
    let chain = chain(&server, youtube.clone());
    chain.set_lossless(true);
    assert_eq!(chain.resolve_track(queen()).await.unwrap().source, "ANDROID_VR");
    assert!(!chain.monochrome().is_up());
    let other = StreamRequest { video_id: "another".into(), ..queen() };
    assert_eq!(chain.resolve_track(other).await.unwrap().source, "ANDROID_VR");
    assert_eq!(server.count("/search/tracks"), 1, "no request while resting");
}

#[tokio::test]
async fn concurrent_resolves_share_one_search() {
    let server = catalogs(200).await;
    let chain = chain(&server, Arc::new(FakeYouTube::default()));
    chain.set_lossless(true);
    let (first, second) = tokio::join!(chain.resolve_track(queen()), chain.prefetch_track(queen()));
    assert_eq!(first.unwrap().source, "Monochrome");
    let () = second;
    assert_eq!(server.count("/search/tracks"), 1);
}

#[tokio::test]
async fn prefetch_warms_youtube_only_without_a_match() {
    let server = catalogs(200).await;
    let youtube = Arc::new(FakeYouTube::default());
    let chain = chain(&server, youtube.clone());
    chain.set_lossless(true);
    chain.prefetch_track(queen()).await;
    assert!(youtube.prefetched.lock().unwrap().is_empty());
    let request = StreamRequest { title: "Radio Ga Ga".into(), video_id: "gaga".into(), ..queen() };
    chain.prefetch_track(request).await;
    assert_eq!(*youtube.prefetched.lock().unwrap(), ["gaga"]);
}

/// The chain's Monochrome stream, downloaded and decoded the way the YouTube player does it.
#[test]
fn a_monochrome_stream_downloads_and_decodes() {
    use sf_player::engine::youtube::decode::Decoder;
    use sf_player::engine::youtube::download::{self, DownloadSpec};

    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let (server, stream) = runtime.block_on(async {
        let server = catalogs(200).await;
        let chain = chain(&server, Arc::new(FakeYouTube::default()));
        chain.set_lossless(true);
        let stream = chain.resolve_track(queen()).await.unwrap();
        (server, stream)
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let reader = download::start(
        client,
        runtime.handle(),
        DownloadSpec::new(&stream.url, &stream.user_agent, stream.content_length),
    );
    let mut decoder = Decoder::open(Box::new(reader), &stream.mime).expect("opens as FLAC");
    let mut samples = 0;
    while let Some(chunk) = decoder.next_chunk().unwrap() {
        samples += chunk.len();
    }
    assert_eq!(samples / 2, 44_100, "one second of stereo");
    let track = server.requests().into_iter().find(|request| request.path == "/track/444").expect("downloaded");
    assert_eq!(track.headers["user-agent"], monochrome::USER_AGENT);
}

// ─── JioSaavn as a catalogue ───────────────────────────────────────────────────────────────────────────────

/// JioSaavn only: search answers the recorded results, details the recorded song (or a 500 when `broken`).
async fn saavn_only(broken: bool) -> MockServer {
    MockServer::start(move |request| match request.query.get("__call").map(String::as_str) {
        Some("song.getDetails") if broken => json(500, &Value::Null),
        Some("song.getDetails") => json(200, &fixture("saavn_details.json")),
        _ => json(200, &fixture("saavn_search.json")),
    })
    .await
}

#[tokio::test]
async fn search_gives_saavn_track_items() {
    let server = saavn_only(false).await;
    let source = Saavn::new(http()).with_base_url(&format!("{}/api.php", server.base));
    let tracks = source.search_tracks("kesariya").await.unwrap();
    assert_eq!(tracks.len(), 4);
    assert!(!tracks[0].available, "Pro-only");
    let kesariya = &tracks[3];
    assert_eq!(kesariya.uri, "saavn:track:OqVAbcde");
    assert_eq!(kesariya.track_id, "OqVAbcde");
    assert_eq!(kesariya.title, "Kesariya (From \"Brahmastra\")");
    assert_eq!(kesariya.artists, "Pritam, Arijit Singh, Amitabh Bhattacharya");
    assert_eq!(kesariya.artist_refs.len(), 3);
    assert_eq!(kesariya.duration_ms, 268_000);
    assert_eq!(kesariya.cover_large, "https://c.saavncdn.com/430/Kesariya-500x500.jpg");
    assert_eq!(kesariya.cover, "https://c.saavncdn.com/430/Kesariya-150x150.jpg");
    assert!(kesariya.available);
    assert!(sf_player::engine::youtube::is_lane_track(&kesariya.uri));
    assert!(source.search_tracks("  ").await.unwrap().is_empty());
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn saavn_items_resolve_through_their_details() {
    let server = saavn_only(false).await;
    let youtube = Arc::new(FakeYouTube::default());
    let chain = chain(&server, youtube.clone());
    // the settings do not matter for a song the listener picked
    let track = md3_model::TrackItem { uri: "saavn:track:nourl".into(), title: "Apna Bana Le".into(), ..Default::default() };
    let request = StreamRequest::from_track(&track);
    assert_eq!(request.video_id, "saavn:track:nourl");
    let stream = chain.resolve_track(request.clone()).await.unwrap();
    assert_eq!(stream.url, "https://aac.saavncdn.com/123/0a1b2c3d4e5f_320.mp4");
    assert_eq!((stream.source.as_str(), stream.video_id.as_str()), ("JioSaavn", "saavn:track:nourl"));
    assert_eq!(stream.quality_label(), "JioSaavn · AAC 320");
    assert_eq!(stream.duration_ms, 261_000);
    // cached; the prefetch of the same song asks nothing either
    chain.resolve_track(request.clone()).await.unwrap();
    chain.prefetch_track(request.clone()).await;
    assert_eq!(server.requests().len(), 1);
    let details = &server.requests()[0];
    assert_eq!((details.query["__call"].as_str(), details.query["pids"].as_str()), ("song.getDetails", "nourl"));
    // a failed URL asks again, and never goes to YouTube
    chain.invalidate("saavn:track:nourl");
    chain.set_data_saver(true);
    assert!(chain.resolve_track(request).await.unwrap().url.ends_with("_96.mp4"));
    assert_eq!(server.requests().len(), 2);
    assert!(youtube.resolved.lock().unwrap().is_empty() && youtube.invalidated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_or_unreachable_saavn_items_fail_with_a_message() {
    let server = saavn_only(false).await;
    let working = chain(&server, Arc::new(FakeYouTube::default()));
    let missing = working.resolve_track(StreamRequest::video("saavn:track:gone")).await.unwrap_err();
    assert!(!missing.is_empty());

    let server = saavn_only(true).await;
    let broken = chain(&server, Arc::new(FakeYouTube::default()));
    let down = broken.resolve_track(StreamRequest::video("saavn:track:nourl")).await.unwrap_err();
    assert!(!down.is_empty() && down != missing);
}

#[test]
fn saavn_songs_follow_the_catalogue_tracks() {
    use super::with_saavn_tracks;
    use crate::ServiceError;
    use crate::catalog::SearchResults;
    let item = |uri: &str| md3_model::TrackItem { uri: uri.into(), ..Default::default() };
    let spotify = SearchResults { tracks: vec![item("spotify:track:a")], ..Default::default() };
    let merged = with_saavn_tracks(Ok(spotify), vec![item("saavn:track:b")]).unwrap();
    let uris: Vec<&str> = merged.tracks.iter().map(|track| track.uri.as_str()).collect();
    assert_eq!(uris, ["spotify:track:a", "saavn:track:b"]);
    let alone = with_saavn_tracks(Err(ServiceError::Other("down".into())), vec![item("saavn:track:b")]).unwrap();
    assert_eq!(alone.tracks.len(), 1);
    assert!(with_saavn_tracks(Err(ServiceError::Other("down".into())), Vec::new()).is_err());
}
