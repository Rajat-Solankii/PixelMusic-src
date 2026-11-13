//! YouTube Music through the app services (Phase 4): sign-in with pasted cookies, restore, sign-out, library
//! pages, likes and edits, catalog routing and radio. Everything runs against an in-process HTTP server and an
//! in-memory credential store: nothing here talks to YouTube, Spotify or the OS keyring.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use md3_model::{ListChange, PlaylistId, TrackItem};
use serde_json::{Value, json};
use sf_api::{ApiGateway, NetActivity};
use sf_player::{PlayerConfig, PlayerHandle};
use sf_services::youtube::InnerTube;
use sf_services::{
    AuthService, CatalogService, HistoryService, LibraryEvent, LibraryKind, LibraryService, YouTubeAuthEvent,
    YouTubeAuthState, YouTubeService,
};
use mf_platform::SettingsStore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/youtube/");
const COOKIES: &str = "SAPISID=test-sapisid; SID=test-sid";

fn fixture(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURES}{name}")).unwrap()).unwrap()
}

#[derive(Clone, Debug)]
struct Request {
    /// path and query
    target: String,
    body: Value,
}

struct MockServer {
    base: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl MockServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let Some(request) = read_request(&mut socket).await else { return };
                    let text = if request.target.starts_with("/sw.js_data") {
                        ")]}'
[[\"x\",1,[\"en\",\"CgtANONYMOUS\"]]]".to_owned()
                    } else {
                        answer(&request).to_string()
                    };
                    recorded.lock().unwrap().push(request);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self { base, requests }
    }

    fn requests_to(&self, endpoint: &str) -> Vec<Request> {
        self.requests.lock().unwrap().iter().filter(|request| request.target.contains(endpoint)).cloned().collect()
    }

    fn youtube(&self) -> Arc<YouTubeService> {
        let http = sf_api::Http::new(reqwest::Client::builder().no_proxy().build().unwrap());
        let client = InnerTube::new(http).with_base_url(&self.base).with_retry_delay(Duration::from_millis(1));
        Arc::new(YouTubeService::with_client(client))
    }
}

fn answer(request: &Request) -> Value {
    let target = request.target.as_str();
    if target.contains("account/account_menu") {
        return json!({ "actions": [{ "openPopupAction": { "popup": { "multiPageMenuRenderer": { "header": {
            "activeAccountHeaderRenderer": {
                "accountName": { "runs": [{ "text": "Test Listener" }] },
                "email": { "runs": [{ "text": "listener@example.test" }] },
                "accountPhoto": { "thumbnails": [{ "url": "https://example.test/face.jpg", "width": 88 }] }
            }
        }}}}}]});
    }
    if target.contains("edit_playlist") {
        return json!({ "status": "STATUS_SUCCEEDED" });
    }
    if target.contains("playlist/create") {
        return json!({ "playlistId": "PLmirror" });
    }
    if target.contains("/browse") {
        if request.body.get("continuation").is_some() {
            return fixture("playlist_tracks_page2.json");
        }
        if request.body["browseId"].as_str().is_some_and(|id| id.starts_with("VL")) {
            return fixture("playlist_tracks_page1.json");
        }
    }
    json!({})
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let target = lines.next()?.split(' ').nth(1)?.to_owned();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers.get("content-length").and_then(|value| value.parse().ok()).unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Some(Request { target, body: serde_json::from_slice(&body).unwrap_or(Value::Null) })
}

struct Fixture {
    dir: PathBuf,
    settings: Arc<SettingsStore>,
    player: PlayerHandle,
    store: sf_auth::Store,
    server: MockServer,
}

async fn fixture_services() -> Fixture {
    let rt = tokio::runtime::Handle::current();
    let dir = std::env::temp_dir().join(format!("sf-services-youtube-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let http = sf_api::Http::default();
    let api = Arc::new(ApiGateway::new(http.clone(), Arc::new(NetActivity::default())));
    let config = PlayerConfig { cover_dir: dir.join("covers"), session_file: None, volume: 0.5, http };
    let player = PlayerHandle::start(&rt, config, api).unwrap();
    let store = sf_auth::Store::in_memory(dir.join("state"));
    Fixture { settings: SettingsStore::in_memory(), player, store, server: MockServer::start().await, dir }
}

impl Fixture {
    /// An AuthService over the mock server; `youtube` lets a restart share or replace the service.
    fn auth(&self, youtube: Arc<YouTubeService>) -> Arc<AuthService> {
        let http = sf_api::Http::default();
        let api = Arc::new(ApiGateway::new(http.clone(), Arc::new(NetActivity::default())));
        AuthService::with_youtube(
            tokio::runtime::Handle::current(),
            self.settings.clone(),
            api,
            http,
            self.player.clone(),
            self.store.clone(),
            youtube,
            self.dir.join("youtube-profile"),
        )
    }
}

async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pasted_cookies_sign_in_save_restore_and_sign_out() {
    let f = fixture_services().await;
    let auth = f.auth(f.server.youtube());
    let mut events = auth.subscribe_youtube();

    let refused = auth.youtube_sign_in_with_cookies("SID=only-sid").unwrap_err();
    assert!(!refused.is_empty());
    assert_eq!(auth.youtube_snapshot().state, YouTubeAuthState::SignedOut);
    assert_eq!(auth.youtube_snapshot().error_text, refused);

    auth.youtube_sign_in_with_cookies(&format!("Cookie: {COOKIES}")).unwrap();
    wait_for("sign-in", || auth.is_youtube_signed_in()).await;
    let card = auth.youtube_snapshot().account.unwrap();
    assert_eq!((card.name.as_str(), card.email.as_str()), ("Test Listener", "listener@example.test"));
    assert!(matches!(events.recv().await.unwrap(), YouTubeAuthEvent::SignedIn(_)));
    assert!(auth.youtube().signed_in());
    assert_eq!(f.server.requests_to("account/account_menu").len(), 1);

    // a restart restores the saved account, card included, and checks it again
    let restarted = f.auth(f.server.youtube());
    restarted.restore_youtube();
    wait_for("restore", || restarted.is_youtube_signed_in()).await;
    assert_eq!(restarted.youtube_snapshot().account.unwrap().name, "Test Listener");
    assert!(restarted.youtube().signed_in());

    restarted.youtube_sign_out();
    assert_eq!(restarted.youtube_snapshot().state, YouTubeAuthState::SignedOut);
    assert!(!restarted.youtube().signed_in());
    let again = f.auth(f.server.youtube());
    again.restore_youtube();
    wait_for("restore after sign-out", || again.youtube_snapshot().state == YouTubeAuthState::SignedOut).await;
    assert!(!again.youtube().signed_in());
    let _ = std::fs::remove_dir_all(&f.dir);
}

fn library(f: &Fixture, auth: &Arc<AuthService>) -> Arc<LibraryService> {
    let rt = tokio::runtime::Handle::current();
    let history = HistoryService::with_file(rt.clone(), auth.clone(), f.player.clone(), f.dir.join("history.json"));
    let library = LibraryService::new(rt, auth.clone(), f.player.clone(), history, f.settings.clone());
    library.use_local_playlists_file(f.dir.join("local_playlists.json"));
    library
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn youtube_playlists_open_page_by_page_and_likes_and_adds_go_to_youtube() {
    let f = fixture_services().await;
    let auth = f.auth(f.server.youtube());
    let library = library(&f, &auth);
    let mut events = library.subscribe();
    auth.youtube_sign_in_with_cookies(COOKIES).unwrap();
    wait_for("sign-in", || auth.is_youtube_signed_in()).await;
    // the library lists Liked Music once the account is in
    wait_for("library", || !library.youtube_playlists().is_empty() && !library.youtube_loading()).await;
    let entries = library.entries(LibraryKind::Playlists, "all", "");
    assert!(entries.iter().any(|entry| entry.uri == "youtube:playlist:LM"), "{entries:?}");

    let id = PlaylistId::from_uri_or_id("youtube:playlist:PL1234567890abcdef");
    library.load_playlist(id.clone());
    wait_for("playlist", || library.playlist_tracks(&id).is_some_and(|rows| rows.len() == 2)).await;
    let rows = library.playlist_tracks(&id).unwrap();
    assert_eq!(rows[0].uri, "youtube:track:vid_track_001");
    let mut changes = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let LibraryEvent::PlaylistTracks { id: changed, change } = event
            && changed == id
        {
            changes.push(change);
        }
    }
    wait_for("loaded", || library.playlist_info(&id).is_some()).await;
    assert!(changes.iter().any(|change| matches!(change, ListChange::Reset(rows) if rows.len() == 1)), "{changes:?}");
    assert!(changes.iter().any(|change| matches!(change, ListChange::Append(rows) if rows.len() == 1)), "{changes:?}");
    let browse = f.server.requests_to("/browse");
    assert!(browse.iter().any(|request| request.body["browseId"] == "VLPL1234567890abcdef"));

    // like: YouTube is told, the liked set changes at once
    assert!(library.is_liked("youtube:track:vid_track_001"), "Liked Music was read");
    let track = TrackItem { uri: "youtube:track:fresh_video".into(), title: "Song".into(), ..Default::default() };
    assert!(!library.is_liked(&track.uri));
    library.toggle_like(track.clone());
    assert!(library.is_liked(&track.uri));
    wait_for("like request", || !f.server.requests_to("like/like").is_empty()).await;
    assert_eq!(f.server.requests_to("like/like")[0].body["target"]["videoId"], "fresh_video");

    // adds: Spotify songs are left out, YouTube ones go to edit_playlist
    library.add_tracks(id.clone(), vec!["spotify:track:abc".into(), "youtube:track:newvideo".into()]);
    wait_for("edit request", || !f.server.requests_to("edit_playlist").is_empty()).await;
    let edit = &f.server.requests_to("edit_playlist")[0].body;
    assert_eq!(edit["playlistId"], "PL1234567890abcdef");
    assert_eq!(edit["actions"].as_array().unwrap().len(), 1, "{edit}");
    assert_eq!(edit["actions"][0]["addedVideoId"], "newvideo");

    // Spotify playlists still go the Spotify way: nothing reaches the YouTube server
    let before = f.server.requests.lock().unwrap().len();
    assert!(library.playlist_tracks(&PlaylistId::new("37i9dQZF1DXcBWIGoYBM5M")).is_none());
    assert_eq!(f.server.requests.lock().unwrap().len(), before);
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn youtube_uris_route_to_youtube_and_need_an_account_for_likes() {
    let f = fixture_services().await;
    let auth = f.auth(f.server.youtube());
    let library = library(&f, &auth);
    let mut events = library.subscribe();
    // signed out of YouTube: a like is refused with a notice and nothing is sent
    library.toggle_like(TrackItem { uri: "youtube:track:v1".into(), ..Default::default() });
    assert!(!library.is_liked("youtube:track:v1"));
    let notice = loop {
        match events.recv().await.unwrap() {
            LibraryEvent::Notice(text) => break text,
            _ => continue,
        }
    };
    assert!(!notice.is_empty());
    assert!(f.server.requests_to("like/").is_empty());

    // the catalog sends youtube: album URIs to YouTube (an empty answer is a parse error, not a Spotify call)
    let catalog = CatalogService::new(auth.clone(), f.player.clone());
    let album = catalog.album("youtube:album:MPREb_test").await;
    assert!(album.is_err());
    assert!(f.server.requests_to("/browse").iter().any(|request| request.body["browseId"] == "MPREb_test"));
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_playlists_mirror_their_edits_to_youtube_music() {
    let f = fixture_services().await;
    let auth = f.auth(f.server.youtube());
    let library = library(&f, &auth);
    auth.youtube_sign_in_with_cookies(COOKIES).unwrap();
    wait_for("sign-in", || auth.is_youtube_signed_in()).await;
    wait_for("library", || !library.youtube_loading()).await;
    assert!(f.settings.get_bool(mf_platform::SettingKey::LocalPlaylistSync), "on by default");

    // a new local playlist gets a private mirror with its YouTube songs (the local file is left out)
    let id = library.create_local_playlist(
        "Mirror me".into(),
        vec!["youtube:track:a".into(), "local:track:00ff".into(), "youtube:track:b".into()],
        None,
    );
    wait_for("mirror created", || library.local_playlists()[0].youtube_id.as_deref() == Some("PLmirror")).await;
    let create = &f.server.requests_to("playlist/create")[0].body;
    assert_eq!((create["title"].as_str(), create["privacyStatus"].as_str()), (Some("Mirror me"), Some("PRIVATE")));
    assert_eq!(create["videoIds"], json!(["a", "b"]));
    assert!(!library.local_playlists()[0].dirty);
    let entries = library.entries(LibraryKind::Playlists, "all", "");
    assert!(entries.iter().any(|entry| entry.uri == id.as_str()));
    assert!(!entries.iter().any(|entry| entry.uri == "youtube:playlist:PLmirror"), "the mirror is not listed twice");

    // an edit: the mirror is read and brought in line (its other songs out, the new one in)
    library.add_tracks(id.clone(), vec!["youtube:track:c".into()]);
    wait_for("edit synced", || !library.local_playlists()[0].dirty).await;
    assert!(f.server.requests_to("/browse").iter().any(|request| request.body["browseId"] == "VLPLmirror"));
    let edits: Vec<Value> = f.server.requests_to("edit_playlist").into_iter().map(|request| request.body).collect();
    let actions: Vec<&Value> = edits.iter().flat_map(|edit| edit["actions"].as_array().unwrap()).collect();
    assert!(actions.iter().any(|action| action["action"] == "ACTION_REMOVE_VIDEO"), "{edits:?}");
    let added: Vec<&str> = actions.iter().filter_map(|action| action["addedVideoId"].as_str()).collect();
    assert_eq!(added, ["a", "b", "c"]);

    // the setting off: edits stay local (and wait for the next sync)
    f.settings.set(mf_platform::SettingKey::LocalPlaylistSync, mf_platform::settings::SettingValue::Bool(false));
    let before = f.server.requests.lock().unwrap().len();
    library.rename_playlist(id.clone(), "Offline".into());
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(f.server.requests.lock().unwrap().len(), before);
    assert!(library.local_playlists()[0].dirty);
    let _ = std::fs::remove_dir_all(&f.dir);
}
