//! Hermetic: a local mock of ws.audioscrobbler.com, `Store::in_memory`, `SettingsStore::in_memory`, a manual clock.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use md3_model::ArtistRef;
use mf_platform::SettingValue;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

const KEY: &str = "dummy-api-key";
const SECRET: &str = "dummy-shared-secret";
const SESSION: &str = "dummy-session-key";
const START: i64 = 1_700_000_000;

type Params = HashMap<String, String>;
type Handler = Arc<dyn Fn(&Params, usize) -> Value + Send + Sync>;

struct MockServer {
    url: String,
    requests: Arc<Mutex<Vec<Params>>>,
}

impl MockServer {
    /// `answer(params, n)` — n: how many earlier requests had the same method
    async fn start(answer: impl Fn(&Params, usize) -> Value + Send + Sync + 'static) -> Self {
        let answer: Handler = Arc::new(answer);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/2.0/", listener.local_addr().unwrap());
        let requests: Arc<Mutex<Vec<Params>>> = Arc::default();
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (recorded, answer) = (recorded.clone(), answer.clone());
                tokio::spawn(async move {
                    let Some(params) = read_form(&mut socket).await else { return };
                    let value = {
                        let mut recorded = recorded.lock().unwrap();
                        let n = recorded.iter().filter(|p| p.get("method") == params.get("method")).count();
                        recorded.push(params.clone());
                        answer(&params, n)
                    };
                    let text = value.to_string();
                    let status = if value.get("error").is_some() { "400 Bad Request" } else { "200 OK" };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self { url, requests }
    }

    fn calls(&self, method: &str) -> Vec<Params> {
        self.requests.lock().unwrap().iter().filter(|p| p.get("method").is_some_and(|m| m == method)).cloned().collect()
    }
}

async fn read_form(socket: &mut tokio::net::TcpStream) -> Option<Params> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_ascii_lowercase();
    assert!(head.starts_with("post /2.0/ "), "{head}");
    assert!(head.contains("content-type: application/x-www-form-urlencoded"), "{head}");
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    while buffer.len() < head_end + length {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8(buffer[head_end..].to_vec()).ok()?;
    Some(
        body.split('&')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let (name, value) = part.split_once('=').unwrap_or((part, ""));
                (url_decode(name), url_decode(value))
            })
            .collect(),
    )
}

fn url_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

/// api_sig of a recorded request matches its other parameters (format and api_sig itself unsigned).
fn assert_signed(params: &Params, secret: &str) {
    let signed: Vec<(String, String)> = params
        .iter()
        .filter(|(name, _)| *name != "api_sig" && *name != "format")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    assert_eq!(params["api_sig"], signature(&signed, secret));
    assert_eq!(params["format"], "json");
}

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!("materialfy-lastfm-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    server: MockServer,
    settings: Arc<SettingsStore>,
    store: Store,
    service: Arc<LastFmService>,
    clock: Arc<Mutex<Instant>>,
    unix: Arc<AtomicI64>,
    opened: Arc<Mutex<Vec<String>>>,
    _dir: Dir,
}

impl Harness {
    async fn new(saved: Option<LastFmAccount>, answer: impl Fn(&Params, usize) -> Value + Send + Sync + 'static) -> Self {
        Self::build(saved, None, None, answer).await
    }

    /// `queue_file` — shared between runs (None: one in this harness's own folder); `api_url` — another server
    async fn build(
        saved: Option<LastFmAccount>,
        queue_file: Option<PathBuf>,
        api_url: Option<String>,
        answer: impl Fn(&Params, usize) -> Value + Send + Sync + 'static,
    ) -> Self {
        let server = MockServer::start(answer).await;
        let dir = Dir::new();
        let queue_file = queue_file.unwrap_or_else(|| dir.0.join("lastfm-queue.json"));
        let store = Store::in_memory(dir.0.clone());
        if let Some(account) = saved {
            store.lease(Slot::LastFm).save(StoredGrant::LastFm(account)).await.unwrap();
        }
        let settings = SettingsStore::in_memory();
        let clock = Arc::new(Mutex::new(Instant::now()));
        let unix = Arc::new(AtomicI64::new(START));
        let opened: Arc<Mutex<Vec<String>>> = Arc::default();
        let options = LastFmOptions {
            api_url: api_url.unwrap_or_else(|| server.url.clone()),
            queue_file: Some(queue_file),
            retry_delay: Duration::from_millis(30),
            auth_url: "https://auth.example.test/api/auth/".into(),
            sign_in_poll: Duration::from_millis(10),
            sign_in_timeout: Duration::from_secs(5),
            open_browser: {
                let opened = opened.clone();
                Arc::new(move |url: &str| {
                    opened.lock().unwrap().push(url.to_owned());
                    Ok(())
                })
            },
            now: {
                let clock = clock.clone();
                Arc::new(move || *clock.lock().unwrap())
            },
            unix_now: {
                let unix = unix.clone();
                Arc::new(move || unix.load(Ordering::SeqCst))
            },
        };
        let http = sf_api::Http::new(reqwest::Client::builder().no_proxy().build().unwrap());
        let service = LastFmService::with_options(Handle::current(), settings.clone(), http, store.clone(), options);
        let harness = Self { server, settings, store, service, clock, unix, opened, _dir: dir };
        harness.wait(|s| s.loaded).await;
        harness
    }

    fn advance(&self, by: Duration) {
        *self.clock.lock().unwrap() += by;
    }

    async fn wait(&self, done: impl Fn(&LastFmSnapshot) -> bool) {
        for _ in 0..500 {
            if done(&self.service.snapshot()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out: {:?}", self.service.snapshot());
    }

    async fn wait_calls(&self, method: &str, count: usize) -> Vec<Params> {
        for _ in 0..500 {
            let calls = self.server.calls(method);
            if calls.len() >= count {
                return calls;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{method}: expected {count} calls, got {:?}", self.server.calls(method));
    }

    /// Nothing more arrives for a moment.
    async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    async fn saved(&self) -> Option<LastFmAccount> {
        match self.store.lease(Slot::LastFm).load().await.unwrap().grant {
            Some(StoredGrant::LastFm(account)) => Some(account),
            None => None,
            Some(_) => panic!("another grant in the Last.fm slot"),
        }
    }
}

fn connected() -> LastFmAccount {
    LastFmAccount {
        api_key: KEY.into(),
        secret: SECRET.into(),
        session_key: SESSION.into(),
        username: "dummy-listener".into(),
    }
}

fn track(uri: &str, artist: &str, title: &str, seconds: i64) -> TrackItem {
    TrackItem {
        title: title.into(),
        artists: artist.into(),
        artist_refs: vec![ArtistRef { id: "a1".into(), name: artist.into() }],
        album: "Dummy Album".into(),
        duration_ms: seconds * 1000,
        uri: uri.into(),
        ..Default::default()
    }
}

fn ok(_: &Params, _: usize) -> Value {
    json!({ "scrobbles": { "@attr": { "accepted": 1, "ignored": 0 } } })
}

// --- pure parts ---

#[test]
fn signature_is_md5_of_sorted_params_and_secret() {
    assert_eq!(signature(&[], ""), "d41d8cd98f00b204e9800998ecf8427e");
    // sorted: "a"+"b", then "c"+"" — md5("abc")
    assert_eq!(signature(&[pair("c", ""), pair("a", "b")], ""), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(signature(&[pair("a", "b")], "c"), "900150983cd24fb0d6963f7d28e17f72");
}

#[test]
fn form_encoding_escapes_everything_but_unreserved() {
    assert_eq!(
        form_encode(&[pair("artist[0]", "AC/DC & Co"), pair("track", "Ünï-code_ok.~")]),
        "artist%5B0%5D=AC%2FDC%20%26%20Co&track=%C3%9Cn%C3%AF-code_ok.~"
    );
}

#[test]
fn scrobble_rule_thresholds() {
    let s = Duration::from_secs;
    assert_eq!(scrobble_after(None), Some(s(180)));
    assert_eq!(scrobble_after(Some(s(30))), None);
    assert_eq!(scrobble_after(Some(s(10))), None);
    assert_eq!(scrobble_after(Some(s(31))), Some(Duration::from_millis(15_500)));
    assert_eq!(scrobble_after(Some(s(240))), Some(s(120)));
    assert_eq!(scrobble_after(Some(s(360))), Some(s(180)));
    assert_eq!(scrobble_after(Some(s(3600))), Some(s(180)));
}

#[test]
fn played_time_pauses_with_playback() {
    let s = Duration::from_secs;
    let t0 = Instant::now();
    let mut listen = Listen::new(Song::from_track(&track("u", "Artist", "Song", 200)).unwrap());
    assert_eq!(listen.due_at(), None);
    assert!(listen.set_playing(true, t0, START));
    assert!(!listen.set_playing(true, t0 + s(5), START + 5), "still the same listen");
    assert_eq!(listen.due_at(), Some(t0 + s(100)));
    listen.set_playing(false, t0 + s(40), START + 40);
    assert_eq!(listen.due_at(), None);
    assert!(!listen.take_due(t0 + s(1000)));
    assert!(!listen.set_playing(true, t0 + s(500), START + 500));
    assert_eq!(listen.due_at(), Some(t0 + s(560)));
    assert!(!listen.take_due(t0 + s(559)));
    assert!(listen.take_due(t0 + s(560)));
    assert!(!listen.take_due(t0 + s(900)), "once per listen");
    assert_eq!(listen.started_at, Some(START));

    let mut short = Listen::new(Song::from_track(&track("u", "Artist", "Jingle", 30)).unwrap());
    assert!(short.set_playing(true, t0, START));
    assert_eq!(short.due_at(), None);
    assert!(!short.take_due(t0 + s(3600)));
}

#[test]
fn songs_from_tracks() {
    let mut item = track("u", "First", "  Title ", 200);
    item.artists = "First, Second".into();
    item.artist_refs.push(ArtistRef { id: "a2".into(), name: "Second".into() });
    let song = Song::from_track(&item).unwrap();
    assert_eq!((song.artist.as_str(), song.title.as_str(), song.album.as_str()), ("First", "Title", "Dummy Album"));
    assert_eq!(song.duration, Some(Duration::from_secs(200)));
    // no refs: the first name of "A, B"; YouTube "- Topic" channels
    item.artist_refs.clear();
    assert_eq!(Song::from_track(&item).unwrap().artist, "First");
    item.artists = "Some Band - Topic".into();
    assert_eq!(Song::from_track(&item).unwrap().artist, "Some Band");
    item.duration_ms = 0;
    assert_eq!(Song::from_track(&item).unwrap().duration, None);
    assert!(Song::from_track(&track("u", "Unknown artist", "Title", 200)).is_none());
    assert!(Song::from_track(&track("u", "Artist", " ", 200)).is_none());
    let mut episode = track("spotify:episode:x", "Show", "Episode", 2000);
    episode.is_episode = true;
    assert!(Song::from_track(&episode).is_none());
}

// --- service over the mock server ---

#[tokio::test]
async fn browser_sign_in_saves_the_session() {
    let h = Harness::new(None, |params, n| match params["method"].as_str() {
        "auth.getToken" => json!({ "token": "dummy-token" }),
        // the first poll comes before the click in the browser
        "auth.getSession" if n == 0 => json!({ "error": 14, "message": "Unauthorized Token - This token has not been authorized" }),
        "auth.getSession" => json!({ "session": { "name": "dummy-listener", "key": SESSION, "subscriber": 0 } }),
        _ => json!({ "error": 3, "message": "Invalid Method" }),
    })
    .await;
    let mut events = h.service.subscribe();
    h.service.begin_sign_in();
    h.wait(|s| !s.signing_in).await;
    assert_eq!(h.service.snapshot().error_text, LastFmError::NoKeys.to_string());
    assert!(h.server.requests.lock().unwrap().is_empty());

    h.service.set_keys(&format!(" {KEY} "), SECRET);
    h.service.begin_sign_in();
    h.wait(|s| s.connected).await;
    let snapshot = h.service.snapshot();
    assert_eq!((snapshot.username.as_str(), snapshot.api_key.as_str()), ("dummy-listener", KEY));
    assert!(!snapshot.signing_in && snapshot.error_text.is_empty() && snapshot.auth_url.is_empty());
    assert_eq!(*h.opened.lock().unwrap(), [format!("https://auth.example.test/api/auth/?api_key={KEY}&token=dummy-token")]);
    let token = h.server.calls("auth.getToken");
    let sessions = h.server.calls("auth.getSession");
    assert_eq!((token.len(), sessions.len()), (1, 2));
    for params in token.iter().chain(&sessions) {
        assert_eq!(params["api_key"], KEY);
        assert!(!params.contains_key("sk"));
        assert_signed(params, SECRET);
    }
    assert_eq!(sessions[1]["token"], "dummy-token");
    loop {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap() {
            LastFmEvent::Connected(name) => break assert_eq!(name, "dummy-listener"),
            _ => continue,
        }
    }
    assert!(h.saved().await == Some(connected()));

    // disconnect keeps the keys for the next Connect
    h.service.disconnect();
    assert!(!h.service.snapshot().connected && h.service.snapshot().has_keys);
    let saved = h.saved().await.unwrap();
    assert!(saved.has_keys() && saved.session_key.is_empty() && saved.username.is_empty());
    h.service.forget();
    assert!(!h.service.snapshot().has_keys);
    assert!(h.saved().await.is_none());
}

#[tokio::test]
async fn sign_in_gives_up_when_the_browser_approval_never_comes() {
    let h = Harness::new(Some(LastFmAccount::new(KEY, SECRET)), |params, _| match params["method"].as_str() {
        "auth.getToken" => json!({ "token": "dummy-token" }),
        _ => json!({ "error": 14, "message": "Unauthorized Token" }),
    })
    .await;
    assert!(h.service.snapshot().has_keys && !h.service.snapshot().connected);
    h.service.begin_sign_in();
    h.wait(|s| !s.auth_url.is_empty()).await;
    h.wait_calls("auth.getSession", 2).await;
    h.advance(SIGN_IN_TIMEOUT);
    h.wait(|s| !s.signing_in).await;
    assert_eq!(h.service.snapshot().error_text, LastFmError::TimedOut.to_string());
    assert!(!h.service.snapshot().connected);

    // a rejected key ends it at once, with its own text
    let h = Harness::new(Some(LastFmAccount::new(KEY, SECRET)), |_, _| json!({ "error": 10, "message": "Invalid API key" }))
        .await;
    h.service.begin_sign_in();
    h.wait(|s| !s.signing_in).await;
    assert_eq!(h.service.snapshot().error_text, LastFmError::Api { code: 10, message: String::new() }.to_string());
    assert!(h.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn now_playing_then_one_scrobble_after_half_the_played_time() {
    let h = Harness::new(Some(connected()), ok).await;
    assert!(h.service.snapshot().connected);
    let s = Duration::from_secs;
    h.service.track_started(&track("spotify:track:one", "Dummy Artist", "Dummy Song", 200), true);
    let now = h.wait_calls("track.updateNowPlaying", 1).await;
    assert_signed(&now[0], SECRET);
    assert_eq!(now[0]["sk"], SESSION);
    assert_eq!(now[0]["artist"], "Dummy Artist");
    assert_eq!(now[0]["track"], "Dummy Song");
    assert_eq!(now[0]["album"], "Dummy Album");
    assert_eq!(now[0]["duration"], "200");

    // 60 s played, a long pause, then 39 s more: 99 of the 100 needed
    h.advance(s(60));
    h.service.set_playing(false);
    h.advance(s(300));
    h.unix.store(START + 360, Ordering::SeqCst);
    h.service.set_playing(true);
    h.service.set_playing(true);
    h.advance(s(39));
    h.service.poll();
    h.settle().await;
    assert!(h.server.calls("track.scrobble").is_empty());
    assert_eq!(h.server.calls("track.updateNowPlaying").len(), 1, "resuming is not a new listen");

    h.advance(s(1));
    let mut events = h.service.subscribe();
    h.service.poll();
    let scrobbles = h.wait_calls("track.scrobble", 1).await;
    let params = &scrobbles[0];
    assert_signed(params, SECRET);
    assert_eq!(params["sk"], SESSION);
    assert_eq!(params["artist[0]"], "Dummy Artist");
    assert_eq!(params["track[0]"], "Dummy Song");
    assert_eq!(params["album[0]"], "Dummy Album");
    assert_eq!(params["duration[0]"], "200");
    assert_eq!(params["timestamp[0]"], START.to_string(), "the moment it started playing");
    let event = tokio::time::timeout(s(5), events.recv()).await.unwrap().unwrap();
    assert_eq!(event, LastFmEvent::Scrobbled { artist: "Dummy Artist".into(), title: "Dummy Song".into() });

    h.advance(s(500));
    h.service.poll();
    h.settle().await;
    assert_eq!(h.server.calls("track.scrobble").len(), 1, "once per listen");

    // the same track again is a new listen
    h.service.track_started(&track("spotify:track:one", "Dummy Artist", "Dummy Song", 200), true);
    h.wait_calls("track.updateNowPlaying", 2).await;
    h.advance(s(100));
    h.service.poll();
    h.wait_calls("track.scrobble", 2).await;
}

#[tokio::test]
async fn the_armed_timer_scrobbles_without_polling() {
    let h = Harness::new(Some(connected()), ok).await;
    h.service.track_started(&track("yt:one", "Dummy Artist", "Short But Counted", 31), false);
    // playing since 16 s ago: 15.5 s needed, so the timer's (real) deadline has passed already;
    // the spawned timer runs only at the next await, when the manual clock reads "now" again
    *h.clock.lock().unwrap() = Instant::now() - Duration::from_secs(16);
    h.service.set_playing(true);
    *h.clock.lock().unwrap() = Instant::now();
    h.wait_calls("track.scrobble", 1).await;
}

#[tokio::test]
async fn short_tracks_ended_tracks_and_toggles() {
    let h = Harness::new(Some(connected()), ok).await;
    let s = Duration::from_secs;
    // 30 s: "now playing" yes, scrobble never
    h.service.track_started(&track("yt:jingle", "Dummy Artist", "Jingle", 30), true);
    h.wait_calls("track.updateNowPlaying", 1).await;
    h.advance(s(3600));
    h.service.poll();

    // an ended track is not scrobbled later
    h.service.track_started(&track("yt:two", "Dummy Artist", "Two", 200), true);
    h.wait_calls("track.updateNowPlaying", 2).await;
    h.service.track_ended(&track("yt:two", "Dummy Artist", "Two", 200));
    h.advance(s(3600));
    h.service.poll();

    // podcast episodes are left alone
    let mut episode = track("spotify:episode:e", "Show", "Episode", 3000);
    episode.is_episode = true;
    h.service.track_started(&episode, true);

    // toggles are read when each call would go out
    h.settings.set(SettingKey::LastFmNowPlaying, SettingValue::Bool(false));
    h.settings.set(SettingKey::LastFmScrobble, SettingValue::Bool(false));
    h.service.track_started(&track("yt:three", "Dummy Artist", "Three", 200), true);
    h.advance(s(3600));
    h.service.poll();
    h.settle().await;
    assert_eq!(h.server.calls("track.updateNowPlaying").len(), 2);
    assert!(h.server.calls("track.scrobble").is_empty());

    h.settings.set(SettingKey::LastFmScrobble, SettingValue::Bool(true));
    h.service.track_started(&track("yt:four", "Dummy Artist", "Four", 200), true);
    h.advance(s(100));
    h.service.poll();
    let scrobbles = h.wait_calls("track.scrobble", 1).await;
    assert_eq!(scrobbles[0]["track[0]"], "Four");
    h.settle().await;
    assert_eq!(h.server.calls("track.updateNowPlaying").len(), 2);
}

#[tokio::test]
async fn nothing_goes_out_without_a_session() {
    let h = Harness::new(Some(LastFmAccount::new(KEY, SECRET)), ok).await;
    h.service.track_started(&track("yt:one", "Dummy Artist", "One", 200), true);
    h.advance(Duration::from_secs(3600));
    h.service.poll();
    h.settle().await;
    assert!(h.server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_revoked_session_is_forgotten() {
    let h = Harness::new(Some(connected()), |_, _| json!({ "error": 9, "message": "Invalid session key - Please re-authenticate" }))
        .await;
    h.service.track_started(&track("yt:one", "Dummy Artist", "One", 200), true);
    h.wait(|s| !s.connected).await;
    let snapshot = h.service.snapshot();
    assert!(snapshot.has_keys);
    assert_eq!(snapshot.error_text, LastFmError::Api { code: 9, message: String::new() }.to_string());
    // the write is queued before the snapshot changes, so this read comes after it
    let saved = h.saved().await.unwrap();
    assert!(saved.has_keys() && saved.session_key.is_empty());
    // and the scrobble that would follow is not sent
    h.advance(Duration::from_secs(3600));
    h.service.poll();
    h.settle().await;
    assert!(h.server.calls("track.scrobble").is_empty());
}

#[test]
fn errors_read_as_interface_text() {
    assert!(LastFmError::Api { code: 26, message: "Suspended API key".into() }.to_string().contains("API key"));
    assert_eq!(LastFmError::Api { code: 29, message: "Rate limit exceeded".into() }.to_string(), "Rate limit exceeded");
    assert_eq!(LastFmError::Api { code: 16, message: String::new() }.to_string(), "Last.fm error 16");
}

// --- the queue of scrobbles not sent yet ---

fn pending(user: &str, title: &str, timestamp: i64) -> Pending {
    Pending {
        user: user.into(),
        artist: "Dummy Artist".into(),
        title: title.into(),
        album: "Dummy Album".into(),
        duration_s: 200,
        timestamp,
    }
}

/// Plays a 200 s track until its scrobble is due.
fn listen_through(h: &Harness, uri: &str, title: &str) {
    h.service.track_started(&track(uri, "Dummy Artist", title, 200), true);
    h.advance(Duration::from_secs(100));
    h.service.poll();
}

async fn wait_for(what: &str, done: impl Fn() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out: {what}");
}

#[test]
fn the_queue_batches_prunes_and_survives_a_restart() {
    let dir = Dir::new();
    let path = dir.0.join("nested").join("lastfm-queue.json");
    let mut queue = Queue::load(Some(path.clone()));
    for i in 0..120 {
        queue.push(pending("me", &format!("Song {i}"), START + i));
    }
    queue.push(pending("me", "Song 0", START));
    queue.push(pending("someone-else", "Theirs", START));
    assert_eq!(queue.len(), 121, "the same listen is queued once");

    let batch = queue.batch("me");
    assert_eq!(batch.len(), queue::BATCH);
    assert_eq!((batch[0].title.as_str(), batch[49].timestamp), ("Song 0", START + 49));
    let params: Params = batch_params(&batch[..2]).into_iter().collect();
    assert_eq!(params["artist[1]"], "Dummy Artist");
    assert_eq!(params["track[1]"], "Song 1");
    assert_eq!(params["timestamp[1]"], (START + 1).to_string());
    assert_eq!((params["album[0]"].as_str(), params["duration[0]"].as_str()), ("Dummy Album", "200"));

    let mut queue = Queue::load(Some(path.clone()));
    assert_eq!(queue.items(), Queue::load(Some(path.clone())).items());
    assert_eq!(queue.len(), 121);
    // 14 days on, the first ten are too old for Last.fm
    assert_eq!(queue.prune(START + queue::MAX_AGE_S + 10), 11);
    assert_eq!(queue.batch("me")[0].title, "Song 10");
    queue.remove(&queue.batch("me"));
    assert_eq!(Queue::load(Some(path.clone())).len(), 60);
    while !queue.batch("me").is_empty() {
        queue.remove(&queue.batch("me"));
    }
    assert!(queue.is_empty());
    assert!(!path.exists(), "an empty queue leaves no file");

    // a damaged file is ignored; the size is capped, oldest dropped
    std::fs::write(&path, b"{not json").unwrap();
    let mut memory = Queue::load(Some(path.clone()));
    assert!(memory.is_empty());
    let mut capped = Queue::load(None);
    for i in 0..queue::MAX_QUEUED as i64 + 5 {
        capped.push(pending("me", "Loop", START + i));
    }
    assert_eq!(capped.len(), queue::MAX_QUEUED);
    assert_eq!(capped.items()[0].timestamp, START + 5);
    memory.push(pending("me", "Fresh", START));
    assert_eq!(Queue::load(Some(path)).len(), 1);
}

#[tokio::test]
async fn a_scrobble_that_cannot_reach_last_fm_is_sent_on_the_next_start() {
    let shared = Dir::new();
    let queue_file = shared.0.join("lastfm-queue.json");
    // nothing listens on this port: "error sending request"
    let dead = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}/2.0/", listener.local_addr().unwrap())
    };
    {
        let h = Harness::build(Some(connected()), Some(queue_file.clone()), Some(dead), ok).await;
        listen_through(&h, "yt:offline", "Offline Song");
        assert_eq!(h.service.queued(), 1);
        h.settle().await;
        assert_eq!(h.service.queued(), 1, "kept after the failed sends");
        assert!(queue_file.exists());
    }
    let h = Harness::build(Some(connected()), Some(queue_file.clone()), None, ok).await;
    let scrobbles = h.wait_calls("track.scrobble", 1).await;
    assert_signed(&scrobbles[0], SECRET);
    assert_eq!(scrobbles[0]["track[0]"], "Offline Song");
    assert_eq!(scrobbles[0]["timestamp[0]"], START.to_string());
    assert_eq!(scrobbles[0]["sk"], SESSION);
    wait_for("queue emptied", || h.service.queued() == 0).await;
    wait_for("file removed", || !queue_file.exists()).await;
}

#[tokio::test]
async fn service_offline_is_retried_and_waiting_scrobbles_go_with_the_next() {
    let h = Harness::new(Some(connected()), |params, n| match params["method"].as_str() {
        "track.scrobble" if n == 0 => json!({ "error": 11, "message": "Service Offline - This service is temporarily offline. Try again later." }),
        "track.scrobble" if n == 1 => json!({ "error": 16, "message": "There was a temporary error processing your request." }),
        _ => ok(params, n),
    })
    .await;
    listen_through(&h, "yt:one", "One");
    let calls = h.wait_calls("track.scrobble", 3).await;
    for call in &calls {
        assert_eq!((call["track[0]"].as_str(), call["timestamp[0]"].as_str()), ("One", START.to_string().as_str()));
    }
    wait_for("queue emptied", || h.service.queued() == 0).await;
    h.settle().await;
    assert_eq!(h.server.calls("track.scrobble").len(), 3, "no retries once sent");
}

#[tokio::test]
async fn a_backlog_goes_out_fifty_at_a_time_without_expired_or_foreign_entries() {
    let shared = Dir::new();
    let queue_file = shared.0.join("lastfm-queue.json");
    {
        let mut queue = Queue::load(Some(queue_file.clone()));
        for i in 0..3 {
            queue.push(pending("dummy-listener", &format!("Ancient {i}"), START - 15 * 24 * 3600 + i));
        }
        for i in 0..120 {
            queue.push(pending("dummy-listener", &format!("Song {i}"), START - 3600 + i));
        }
        queue.push(pending("another-account", "Theirs", START - 60));
    }
    let h = Harness::build(Some(connected()), Some(queue_file), None, ok).await;
    let calls = h.wait_calls("track.scrobble", 3).await;
    let sizes: Vec<usize> = calls.iter().map(|call| call.keys().filter(|key| key.starts_with("track[")).count()).collect();
    assert_eq!(sizes, [50, 50, 20]);
    assert_eq!(calls[0]["track[0]"], "Song 0");
    assert_eq!(calls[2]["track[19]"], "Song 119");
    assert!(calls.iter().all(|call| call.values().all(|value| !value.starts_with("Ancient") && value != "Theirs")));
    wait_for("only the other account's left", || h.service.queued() == 1).await;
}

#[tokio::test]
async fn refused_batches_are_dropped_but_a_lost_session_keeps_them() {
    let h = Harness::new(Some(connected()), |params, n| match params["method"].as_str() {
        "track.scrobble" => json!({ "error": 6, "message": "Invalid parameters" }),
        _ => ok(params, n),
    })
    .await;
    listen_through(&h, "yt:bad", "Bad");
    wait_for("refused batch dropped", || h.service.queued() == 0).await;
    h.settle().await;
    assert_eq!(h.server.calls("track.scrobble").len(), 1, "not retried");

    let h = Harness::new(Some(connected()), |params, n| match params["method"].as_str() {
        "track.scrobble" => json!({ "error": 9, "message": "Invalid session key" }),
        _ => ok(params, n),
    })
    .await;
    listen_through(&h, "yt:kept", "Kept");
    h.wait(|s| !s.connected).await;
    h.settle().await;
    assert_eq!(h.service.queued(), 1, "sent after connecting again");
    assert_eq!(h.server.calls("track.scrobble").len(), 1);
}
